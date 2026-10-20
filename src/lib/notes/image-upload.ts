import { ipc } from "@/lib/ipc";

/** 单张图片上限（与 Rust `NOTE_IMAGE_MAX_BYTES` 一致）。 */
const MAX_IMAGE_BYTES = 10 * 1024 * 1024;

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("读取文件失败"));
    reader.readAsDataURL(file);
  });
}

/**
 * 笔记图片上传（Milkdown ImageBlock / plugin-upload 的 `onUpload` 回调）：
 * 校验大小 → 读成 base64 → 后端落盘（内容哈希命名），返回正文应插入的 URL。
 */
export async function saveNoteImageFile(file: File): Promise<string> {
  if (file.size > MAX_IMAGE_BYTES) {
    throw new Error("图片超过 10MB，请压缩后再试");
  }
  const dataUrl = await readAsDataUrl(file);
  const comma = dataUrl.indexOf(",");
  const base64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
  const rawExt = file.type.startsWith("image/") ? file.type.slice(6) : "png";
  const ext = rawExt.toLowerCase() === "jpeg" ? "jpeg" : rawExt.toLowerCase();
  return ipc.importNoteImage(base64, ext);
}
