/**
 * 速记的 Markdown 纯文本助手（移植自 x-hub `utils/markdown.ts`）：
 * 树列表与卡片预览用摘要、新建笔记「首行成标题」用派生标题。
 */

/** 把 Markdown 压成单行纯文本摘要（去掉标记），超长截断加省略号。 */
export function markdownPlainText(markdown: string, max = 120): string {
  let text = markdown
    .replace(/```[\s\S]*?```/g, " ") // 代码块
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ") // 图片
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1") // 链接保留显示文字
    .replace(/^\s{0,3}#{1,6}\s+/gm, "") // 标题
    .replace(/^\s{0,3}>\s?/gm, "") // 引用
    .replace(/^\s{0,3}[-*+]\s+/gm, "") // 无序列表
    .replace(/^\s{0,3}\d+\.\s+/gm, "") // 有序列表
    .replace(/(\*\*|__|~~|`|\*|_)/g, "") // 强调等行内标记
    .replace(/\s+/g, " ")
    .trim();
  if (text.length > max) text = `${text.slice(0, max)}…`;
  return text;
}

/** 由正文首个非空行派生标题（去掉标记后最多 30 字）；无内容时返回空串。 */
export function deriveNoteTitle(markdown: string): string {
  const firstLine = markdown
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  if (!firstLine) return "";
  return markdownPlainText(firstLine, 30);
}
