//! 本地项目的快捷启动：打开项目目录，或**在应用内托管**执行配置好的启动命令。
//!
//! 命令通过 `cmd /c` 启动、不弹控制台窗口（`CREATE_NO_WINDOW`），stdout / stderr
//! 由本模块用管道接住，逐行写入内存环形缓冲，并经 `project:output` 事件推给前端；
//! 运行状态（运行中 / 已停止 / 退出码）经 `project:runs` 事件广播。
//! 「停止」用 `taskkill /T /F` 结束整棵进程树（`cmd` 派生的 node / python 不会随
//! `cmd` 退出），应用真正退出时由 `stop_all` 一并清理。
//!
//! 项目列表本身存在 `AppConfig.projects`（见 `config.rs`），由前端整份读写。
//! 运行记录只活在本次会话里：不落盘，重启应用后从空开始。
//!
//! 一条命令可以有多行（常见于 README 的快速开始），每行按顺序执行；
//! 每条命令还可以带自己的工作目录，同一项目因此能同时挂「前端」和「后端」。

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

/// 运行状态列表变化（载荷是整个列表，量小，前端直接替换）。
pub const RUNS_EVENT: &str = "project:runs";
/// 增量输出（载荷 `{ runId, lines }`）。
pub const OUTPUT_EVENT: &str = "project:output";

/// 不创建控制台窗口（Windows `CREATE_NO_WINDOW`）。
/// 发布版是 GUI 子系统进程，不显式指定的话 `cmd` 会弹出一个可见的黑窗口。
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 单个运行记录最多保留多少行输出（超出丢最旧的）。
const MAX_LOG_LINES: usize = 4000;
/// 单行最大字符数：进度条类输出可能只吐 `\r` 不换行，不截断会无限增长。
const MAX_LINE_CHARS: usize = 4000;
/// 没有换行时待处理字节的上限，超过就当成一行刷出去。
const MAX_PENDING_BYTES: usize = 64 * 1024;

/// 一条运行记录（序列化给前端）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRun {
    pub run_id: String,
    /// 前端给的定位键（`projectId:index`），用来判断「这个按钮对应的命令是否在跑」。
    pub key: String,
    pub project_id: String,
    pub project_name: String,
    pub command_name: String,
    pub command: String,
    pub workdir: String,
    pub running: bool,
    /// 是否是用户点了「停止」（用于把结束文案写成「已停止」）。
    pub stopping: bool,
    pub exit_code: Option<i32>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
}

/// 一行输出。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLine {
    /// `stdout` / `stderr` / `system`。
    pub stream: &'static str,
    pub text: String,
}

struct RunRecord {
    run: ProjectRun,
    /// `cmd.exe` 的 pid（结束进程树时用）。
    pid: u32,
    lines: VecDeque<OutputLine>,
}

/// Tauri 托管的项目运行状态。
pub struct ProjectRuns {
    runs: Mutex<HashMap<String, RunRecord>>,
}

impl ProjectRuns {
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunRecord>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ProjectRuns {
    fn default() -> Self {
        Self::new()
    }
}

/// 校验项目目录：非空且确实是已存在的目录。
fn validate_path(path: &str) -> AppResult<&Path> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::Other("项目目录为空".into()));
    }
    let target = Path::new(trimmed);
    if !target.is_dir() {
        return Err(AppError::Other(format!("项目目录不存在：{trimmed}")));
    }
    Ok(target)
}

/// 把用户粘贴的启动命令整理成 cmd 能一次执行的形式。
///
/// 编辑器里的命令框支持整段粘贴（例如 README 里的快速开始代码块）：
/// - 去掉 Markdown 代码围栏（` ```bash ` / ` ``` `）与行首的 `$ ` 提示符；
/// - 忽略空行与 `#` 开头的注释行；
/// - 多行按顺序执行，用 `&` 连接（某一条失败也继续，最后一条通常是常驻服务）。
///
/// 单行命令原样返回，不改变用户自己写的 `&&` 语义。
fn normalize_command(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix("$ ").unwrap_or(line).trim()
        })
        .filter(|line| !line.is_empty() && !line.starts_with("```") && !line.starts_with('#'))
        .collect();

    match lines.as_slice() {
        [] => String::new(),
        [single] => (*single).to_string(),
        many => many.join(" & "),
    }
}

/// 项目目录可以留空 —— 新项目默认按每条命令自己的工作目录启动。
fn optional_path(path: &str) -> Option<&Path> {
    let trimmed = path.trim();
    (!trimmed.is_empty()).then(|| Path::new(trimmed))
}

/// 解析命令的实际工作目录：`dir` 优先（绝对路径直接用，相对路径按项目目录解析）；
/// `dir` 为空时回落到项目目录。两者都没有就报错，绝不继承应用自身的 cwd。
fn resolve_workdir(project: Option<&Path>, dir: &str) -> AppResult<PathBuf> {
    let dir = dir.trim();
    let target = if dir.is_empty() {
        project
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::Other("这条命令还没选择工作目录".into()))?
    } else {
        let candidate = Path::new(dir);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else if let Some(project) = project {
            project.join(candidate)
        } else {
            return Err(AppError::Other("这条命令还没选择工作目录".into()));
        }
    };
    if !target.is_dir() {
        return Err(AppError::Other(format!(
            "命令目录不存在：{}",
            target.display()
        )));
    }
    Ok(target)
}

/// 校验命令并算出工作目录；真正创建进程之前就该失败的情况都在这里拦下。
fn prepare(path: &str, command: &str, dir: &str) -> AppResult<(String, PathBuf)> {
    let command = normalize_command(command);
    if command.is_empty() {
        return Err(AppError::Other("启动命令为空".into()));
    }
    let workdir = resolve_workdir(optional_path(path), dir)?;
    Ok((command, workdir))
}

/// 在资源管理器里打开项目目录。
pub fn open_folder(app: &AppHandle, path: &str) -> AppResult<()> {
    let target = validate_path(path)?;
    app.opener()
        .open_path(target.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|err| AppError::Other(format!("打开目录失败：{err}")))
}

fn now_secs() -> u64 {
    crate::db::now_secs()
}

/// 生成运行 id：毫秒时间戳 + 进程内自增序号。`uuid` 依赖没开 `v4` 特性，
/// 这里本来也不需要全局唯一，只要本次会话内不重复即可。
fn new_run_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    format!("run-{millis}-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

/// 把一条运行记录的全部输出读出来（供前端打开控制台时兜底拉取）。
pub fn log_of(state: &ProjectRuns, run_id: &str) -> Vec<OutputLine> {
    state
        .lock()
        .get(run_id)
        .map(|record| record.lines.iter().cloned().collect())
        .unwrap_or_default()
}

/// 当前所有运行记录（按启动时间倒序，最新的在前）。
pub fn snapshot(state: &ProjectRuns) -> Vec<ProjectRun> {
    let mut runs: Vec<ProjectRun> = state
        .lock()
        .values()
        .map(|record| record.run.clone())
        .collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    runs
}

fn emit_runs(app: &AppHandle, state: &ProjectRuns) {
    // 广播失败不该影响进程本身：前端还有 get_project_runs 兜底。
    if let Err(err) = app.emit(RUNS_EVENT, snapshot(state)) {
        log::warn!("广播项目运行状态失败：{err}");
    }
}

fn emit_output(app: &AppHandle, run_id: &str, lines: Vec<OutputLine>) {
    if lines.is_empty() {
        return;
    }
    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct Payload<'a> {
        run_id: &'a str,
        lines: &'a [OutputLine],
    }
    if let Err(err) = app.emit(
        OUTPUT_EVENT,
        Payload {
            run_id,
            lines: &lines,
        },
    ) {
        log::warn!("广播项目命令输出失败：{err}");
    }
}

/// 追加输出：先落内存缓冲（供重新打开控制台时回放），再广播增量。
fn push_output(app: &AppHandle, state: &ProjectRuns, run_id: &str, lines: Vec<OutputLine>) {
    if lines.is_empty() {
        return;
    }
    {
        let mut guard = state.lock();
        let Some(record) = guard.get_mut(run_id) else {
            // 记录已被清理（用户点了「清除」）：丢弃迟到的输出。
            return;
        };
        for line in &lines {
            record.lines.push_back(line.clone());
        }
        while record.lines.len() > MAX_LOG_LINES {
            record.lines.pop_front();
        }
    }
    emit_output(app, run_id, lines);
}

/// 去掉 ANSI 控制序列（颜色 / 光标移动 / 清行 / 窗口标题等）。
///
/// Vite、pnpm 这类工具即使输出被管道接走也照样打颜色转义，原样塞进 `<pre>` 里
/// 就会显示成 `[32m`、方块（ESC 字节）——这就是控制台「乱码」的实际来源。
/// 按字节处理（这些序列全是 ASCII），不碰中文字节。
fn strip_ansi(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] != 0x1b {
            out.push(raw[index]);
            index += 1;
            continue;
        }
        // ESC 序列：
        // - `ESC [ … 0x40..=0x7E`：CSI（颜色、清行、光标）
        // - `ESC ] … BEL / ESC \`：OSC（窗口标题等）
        // - `ESC <单字节>`：其他两字节转义
        let Some(next) = raw.get(index + 1).copied() else {
            break;
        };
        match next {
            b'[' => {
                index += 2;
                while index < raw.len() {
                    let byte = raw[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                index += 2;
                while index < raw.len() {
                    if raw[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if raw[index] == 0x1b && raw.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ => index += 2,
        }
    }
    out
}

/// 清洗一行输出：去掉行尾换行符与 ANSI 转义，进度条只保留 `\r` 之后的最后一段；
/// 超长行截断，避免一行撑爆界面。
fn decode_line(raw: &[u8]) -> String {
    let mut end = raw.len();
    if end > 0 && raw[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && raw[end - 1] == b'\r' {
        end -= 1;
    }
    let cleaned = strip_ansi(&raw[..end]);
    // 进度条用裸 `\r` 原地重写：只保留最后一段，否则整条进度历史会挤成一行。
    let body = match cleaned.iter().rposition(|byte| *byte == b'\r') {
        Some(position) => &cleaned[position + 1..],
        None => &cleaned[..],
    };
    let text = String::from_utf8_lossy(body);
    if text.chars().count() > MAX_LINE_CHARS {
        let mut truncated: String = text.chars().take(MAX_LINE_CHARS).collect();
        truncated.push('…');
        truncated
    } else {
        text.into_owned()
    }
}

/// 逐块读取管道，按 `\n` 切行后成批广播（一块 8KB 大约就是一批，避免逐行发事件）。
fn spawn_reader<R: Read + Send + 'static>(
    app: AppHandle,
    run_id: String,
    stream: &'static str,
    mut reader: R,
    pending_count: Arc<AtomicUsize>,
) {
    std::thread::spawn(move || {
        let state = app.state::<ProjectRuns>();
        let mut buffer = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    let mut lines = Vec::new();
                    while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                        let raw: Vec<u8> = pending.drain(..=index).collect();
                        lines.push(OutputLine {
                            stream,
                            text: decode_line(&raw),
                        });
                    }
                    // 长时间没有换行（进度条 / 交互提示）：攒到上限就刷出去，防止无限增长。
                    if pending.len() > MAX_PENDING_BYTES {
                        lines.push(OutputLine {
                            stream,
                            text: decode_line(&pending),
                        });
                        pending.clear();
                    }
                    push_output(&app, &state, &run_id, lines);
                }
                Err(err) => {
                    log::debug!("读取项目命令输出失败 run={run_id} stream={stream}: {err}");
                    break;
                }
            }
        }
        if !pending.is_empty() {
            push_output(
                &app,
                &state,
                &run_id,
                vec![OutputLine {
                    stream,
                    text: decode_line(&pending),
                }],
            );
        }
        // 通知等待线程：这个管道已经排空，可以落定「进程已结束」了。
        pending_count.fetch_sub(1, Ordering::SeqCst);
    });
}

/// 等进程结束，落定状态并广播（`child` 的所有权必须交给等待线程）。
///
/// 退出后还要等两个读取线程把管道里剩下的内容排空（最多 1s），
/// 否则「进程已结束」会插到最后的输出前面。
fn spawn_waiter(app: AppHandle, run_id: String, mut child: Child, pending: Arc<AtomicUsize>) {
    std::thread::spawn(move || {
        let exit_code = match child.wait() {
            Ok(status) => status.code(),
            Err(err) => {
                log::warn!("等待项目命令退出失败 run={run_id}: {err}");
                None
            }
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while pending.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        finish_run(&app, &run_id, exit_code);
    });
}

fn finish_run(app: &AppHandle, run_id: &str, exit_code: Option<i32>) {
    let state = app.state::<ProjectRuns>();
    let tail = {
        let mut guard = state.lock();
        let Some(record) = guard.get_mut(run_id) else {
            return;
        };
        if !record.run.running {
            return;
        }
        record.run.running = false;
        record.run.exit_code = exit_code;
        record.run.ended_at = Some(now_secs());
        let text = if record.run.stopping {
            "进程已停止".to_string()
        } else {
            match exit_code {
                Some(0) => "进程已结束（退出码 0）".to_string(),
                Some(code) => format!("进程已退出（退出码 {code}）"),
                None => "进程已结束".to_string(),
            }
        };
        let line = OutputLine {
            stream: "system",
            text,
        };
        record.lines.push_back(line.clone());
        line
    };
    emit_output(app, run_id, vec![tail]);
    emit_runs(app, &state);
}

/// 结束整棵进程树。`cmd` 自己退出不会带走 node / python 子进程，必须用 `/T`。
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let result = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    match result {
        Ok(status) if status.success() => log::debug!("已结束项目命令进程树 pid={pid}"),
        Ok(status) => log::debug!("结束项目命令进程树 pid={pid} 返回 {status}"),
        Err(err) => log::warn!("结束项目命令进程树失败 pid={pid}: {err}"),
    }
}

/// 用 `cmd` 跑一条命令（不弹窗口、stdout / stderr 接管道）。
///
/// **命令串必须走 `raw_arg`**：`Command::arg` 会按 MSVC 规则给含空格的参数加引号、
/// 并把参数内部的 `"` 转义成 `\"`，而 `cmd.exe` 不认这种转义——`set "VAR=value"`、
/// 带空格的引号路径都会被拆坏（`JAVA_HOME` 设不上，Maven 就会报「JRE rather than a JDK」）。
/// `raw_arg` 原样拼接，交给 cmd 自己解析引号。
fn spawn_shell(workdir: &Path, command: &str) -> AppResult<Child> {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    // `/d` 跳过 AutoRun 注册表命令；`chcp 65001` 让 cmd 自己的输出走 UTF-8。
    cmd.arg("/d")
        .arg("/c")
        .raw_arg(format!("chcp 65001 >nul & {command}"))
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map_err(|err| AppError::Other(format!("启动命令失败：{err}")))
}

/// 启动一条命令并纳入托管，返回新的运行记录。
#[allow(clippy::too_many_arguments)]
pub fn start(
    app: &AppHandle,
    state: &ProjectRuns,
    key: String,
    project_id: String,
    project_name: String,
    command_name: String,
    path: &str,
    command: &str,
    dir: &str,
) -> AppResult<ProjectRun> {
    let (command, workdir) = prepare(path, command, dir)?;

    // 同一个按钮重复点击不该起两份（端口冲突时用户只会看到一堆报错）。
    {
        let guard = state.lock();
        if let Some(existing) = guard
            .values()
            .find(|record| record.run.running && record.run.key == key)
        {
            return Err(AppError::Other(format!(
                "{} 正在运行，先停止再启动",
                existing.run.command_name
            )));
        }
    }

    let mut child = spawn_shell(&workdir, &command)?;

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let reader_count = usize::from(stdout.is_some()) + usize::from(stderr.is_some());
    let pending = Arc::new(AtomicUsize::new(reader_count));
    let run_id = new_run_id();
    let workdir_text = workdir.display().to_string();

    let run = ProjectRun {
        run_id: run_id.clone(),
        key,
        project_id,
        project_name,
        command_name,
        command: command.clone(),
        workdir: workdir_text.clone(),
        running: true,
        stopping: false,
        exit_code: None,
        started_at: now_secs(),
        ended_at: None,
    };

    {
        let mut lines = VecDeque::new();
        lines.push_back(OutputLine {
            stream: "system",
            text: format!("$ {command}"),
        });
        lines.push_back(OutputLine {
            stream: "system",
            text: format!("目录：{workdir_text}"),
        });
        let mut guard = state.lock();
        guard.insert(
            run_id.clone(),
            RunRecord {
                run: run.clone(),
                pid,
                lines,
            },
        );
    }
    emit_runs(app, state);

    if let Some(stdout) = stdout {
        spawn_reader(
            app.clone(),
            run_id.clone(),
            "stdout",
            stdout,
            pending.clone(),
        );
    }
    if let Some(stderr) = stderr {
        spawn_reader(
            app.clone(),
            run_id.clone(),
            "stderr",
            stderr,
            pending.clone(),
        );
    }
    spawn_waiter(app.clone(), run_id.clone(), child, pending);

    log::info!("启动项目命令 run={run_id} pid={pid} cwd={workdir_text} command={command}");
    Ok(run)
}

/// 停止一条运行中的命令（结束整棵进程树）。
pub fn stop(app: &AppHandle, state: &ProjectRuns, run_id: &str) -> AppResult<()> {
    let (pid, command_name) = {
        let mut guard = state.lock();
        let Some(record) = guard.get_mut(run_id) else {
            return Err(AppError::Other("运行记录不存在".into()));
        };
        if !record.run.running {
            return Ok(());
        }
        record.run.stopping = true;
        (record.pid, record.run.command_name.clone())
    };
    kill_tree(pid);
    emit_runs(app, state);
    log::info!("停止项目命令 run={run_id} command={command_name} pid={pid}");
    Ok(())
}

/// 删除一条运行记录（只能删已结束的，正在跑的必须先停止）。
pub fn clear(app: &AppHandle, state: &ProjectRuns, run_id: &str) -> AppResult<()> {
    {
        let mut guard = state.lock();
        match guard.get(run_id) {
            Some(record) if record.run.running => {
                return Err(AppError::Other("命令还在运行，先停止再清除".into()));
            }
            Some(_) => {}
            None => return Ok(()),
        }
        guard.remove(run_id);
    }
    emit_runs(app, state);
    Ok(())
}

/// 清空一条运行记录的输出缓冲（不清运行记录本身）。
pub fn clear_log(state: &ProjectRuns, run_id: &str) {
    if let Some(record) = state.lock().get_mut(run_id) {
        record.lines.clear();
    }
}

/// 应用退出时调用：把所有还活着的命令进程树一并结束，避免留下孤儿进程。
pub fn stop_all(app: &AppHandle) {
    let state = app.state::<ProjectRuns>();
    let targets: Vec<(String, u32)> = {
        let mut guard = state.lock();
        guard
            .values_mut()
            .filter(|record| record.run.running)
            .map(|record| {
                record.run.stopping = true;
                (record.run.run_id.clone(), record.pid)
            })
            .collect()
    };
    if targets.is_empty() {
        return;
    }
    for (run_id, pid) in &targets {
        kill_tree(*pid);
        log::info!("退出应用：结束项目命令 run={run_id} pid={pid}");
    }
    // 给 `taskkill` 一点时间收尾；退出流程随后就走，不会再读状态。
    std::thread::sleep(Duration::from_millis(80));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_missing_paths_are_rejected() {
        assert!(validate_path("").is_err());
        assert!(validate_path("   ").is_err());
        assert!(validate_path(r"D:\prism3-definitely-missing-dir").is_err());
    }

    #[test]
    fn temp_dir_is_a_valid_target() {
        let dir = std::env::temp_dir();
        assert!(validate_path(&dir.to_string_lossy()).is_ok());
    }

    #[test]
    fn empty_command_is_rejected_before_spawning() {
        // 空命令应在真正创建进程之前就失败，避免起一个什么都不做的进程。
        let dir = std::env::temp_dir();
        assert!(prepare(&dir.to_string_lossy(), "   ", "").is_err());
    }

    #[test]
    fn single_line_command_is_kept_as_is() {
        assert_eq!(normalize_command("  pnpm dev  "), "pnpm dev");
        assert_eq!(
            normalize_command("cd backend && uv run uvicorn app.main:app"),
            "cd backend && uv run uvicorn app.main:app"
        );
    }

    #[test]
    fn multiline_command_runs_lines_in_order() {
        // 从 README 整段粘贴时换行不能丢，否则会变成 `uv sync uv run …` 并报参数错误。
        let raw =
            "uv sync\r\nuv run pytest\r\n\r\nuv run uvicorn app.main:app --reload --port 8000\r\n";
        assert_eq!(
            normalize_command(raw),
            "uv sync & uv run pytest & uv run uvicorn app.main:app --reload --port 8000"
        );
    }

    #[test]
    fn fences_comments_and_prompts_are_stripped() {
        let raw = "```bash\n# 同步依赖\n$ uv sync\nuv run pytest\n```\n";
        assert_eq!(normalize_command(raw), "uv sync & uv run pytest");
        assert_eq!(normalize_command("```\n\n```"), "");
    }

    #[test]
    fn workdir_prefers_the_command_dir_and_falls_back_to_the_project() {
        let project = std::env::temp_dir();
        let resolved = resolve_workdir(Some(&project), "   ").expect("空目录应回落到项目目录");
        assert_eq!(resolved, project);

        // 临时目录的上一级必定存在，用它验证相对路径确实按项目目录解析。
        let relative = resolve_workdir(Some(&project), "..").expect("相对路径应可解析");
        assert_eq!(relative, project.join(".."));

        let absolute =
            resolve_workdir(None, &project.to_string_lossy()).expect("绝对路径不需要项目目录");
        assert_eq!(absolute, project);

        // 项目目录为空且命令没选目录：不继承应用自身 cwd，直接报错。
        assert!(resolve_workdir(None, "").is_err());
        assert!(resolve_workdir(None, "..").is_err());
        assert!(resolve_workdir(Some(&project), "prism3-definitely-missing-subdir").is_err());
        assert!(prepare("", "echo hi", "prism3-definitely-missing-subdir").is_err());
    }

    #[test]
    fn output_lines_drop_crlf_and_truncate_long_lines() {
        assert_eq!(decode_line(b"hello\r\n"), "hello");
        assert_eq!(decode_line(b"hello"), "hello");
        assert_eq!(decode_line(b""), "");

        let long = "汉".repeat(MAX_LINE_CHARS + 10);
        let decoded = decode_line(long.as_bytes());
        assert_eq!(decoded.chars().count(), MAX_LINE_CHARS + 1);
        assert!(decoded.ends_with('…'));
    }

    #[test]
    fn output_keeps_utf8_text_intact() {
        assert_eq!(decode_line("构建成功\n".as_bytes()), "构建成功");
    }

    #[test]
    fn ansi_color_sequences_are_stripped() {
        // Vite 即使输出被管道接走也会打颜色转义，原样显示就是「乱码」。
        assert_eq!(
            decode_line(b"\x1b[32m\xe2\x9e\x9c\x1b[39m  \x1b[1mLocal\x1b[22m:   http://x"),
            "➜  Local:   http://x"
        );
        assert_eq!(
            decode_line(b"\x1b[2mready in \x1b[0m2642\x1b[22m ms"),
            "ready in 2642 ms"
        );
        // OSC（窗口标题）也要去掉，不能把标题文字留在控制台里。
        assert_eq!(decode_line(b"\x1b]0;vite\x07ready"), "ready");
    }

    #[test]
    fn bare_carriage_return_keeps_the_last_progress_segment() {
        assert_eq!(decode_line(b"progress 10%\rprogress 20%"), "progress 20%");
    }

    #[test]
    fn quoted_set_commands_survive_cmd_parsing() {
        // 回归：`Command::arg` 会把内部引号转义成 `\"`，cmd 不认，导致
        // `set "JAVA_HOME=..."` 设不上变量。必须用 `raw_arg` 原样传命令串。
        // 用不带参数的 `set PREFIX` 回读（它不依赖变量展开，避免同一行
        // `set` + `%VAR%` 在 cmd 里按旧值展开的干扰）。
        let child = spawn_shell(
            &std::env::temp_dir(),
            r#"set "PRISM3_TEST=hello world" & set PRISM3_TEST"#,
        )
        .expect("启动 cmd 失败");
        let output = child.wait_with_output().expect("等待 cmd 结束失败");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("hello world"),
            "命令里的引号被拆坏了，实际输出：{text}"
        );
    }
}
