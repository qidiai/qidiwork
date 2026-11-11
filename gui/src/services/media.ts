export const VIDEO_MIMES: Record<string, string> = {
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

export const MAX_VIDEO_BYTES = 100 * 1024 * 1024;
export const MAX_VIDEO_FRAMES = 6;

export interface VideoFrame {
  /** 采样时间，单位为秒。 */
  timestamp: number;
  /** 无 data URL 前缀的 base64，与图片附件的 data 契约一致。 */
  data: string;
  mimeType: "image/jpeg";
}

export interface VideoAnalysis {
  duration: number;
  frames: VideoFrame[];
}

const STAGE_TIMEOUT_MS = 10_000;
const MAX_FRAME_BYTES = 10 * 1024 * 1024;
const MAX_FRAME_EDGE = 1280;
const CONVERSION_HINT = "请确认视频可以正常播放，或转换为 MP4（H.264 视频编码）后重试。";

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) {
    throw new DOMException("已取消视频画面提取。", "AbortError");
  }
}

function waitForVideo(
  video: HTMLVideoElement,
  signal: AbortSignal | undefined,
  failureMessage: string,
  isReady: () => boolean,
  start: () => void,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const events = ["loadedmetadata", "loadeddata", "durationchange", "seeked", "canplay", "timeupdate"];

    const finish = (error?: unknown) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      clearInterval(poll);
      for (const event of events) video.removeEventListener(event, check);
      video.removeEventListener("error", onError);
      signal?.removeEventListener("abort", check);
      if (error === undefined) resolve();
      else reject(error);
    };
    const onError = () => finish(new Error(failureMessage));
    const check = () => {
      if (settled) return;
      try {
        throwIfAborted(signal);
        if (video.error) throw new Error(failureMessage);
        if (isReady()) finish();
      } catch (error) {
        finish(error);
      }
    };

    const timeout = setTimeout(
      () => finish(new Error(`处理视频超时（10 秒）。${failureMessage}`)),
      STAGE_TIMEOUT_MS,
    );
    // 同位置 seek 未必触发 seeked；后台文档也未必收到视频渲染回调。
    const poll = setInterval(check, 50);
    for (const event of events) video.addEventListener(event, check);
    video.addEventListener("error", onError);
    signal?.addEventListener("abort", check, { once: true });

    try {
      throwIfAborted(signal);
      if (video.error) throw new Error(failureMessage);
      start();
      check();
    } catch (error) {
      if (signal?.aborted) check();
      else finish(new Error(failureMessage, { cause: error }));
    }
  });
}

export async function extractVideoFrames(file: File, signal?: AbortSignal): Promise<VideoAnalysis> {
  throwIfAborted(signal);
  const extension = /\.([^.]+)$/.exec(file.name)?.[1].toLowerCase() ?? "";
  if (!Object.prototype.hasOwnProperty.call(VIDEO_MIMES, extension)) {
    throw new Error("不支持此视频格式，请选择 .mp4、.webm 或 .mov 文件；其他格式请先转换为 MP4。");
  }
  if (file.size === 0) {
    throw new Error("视频文件为空，请重新选择非空的视频文件。");
  }
  if (file.size > MAX_VIDEO_BYTES) {
    throw new Error("视频文件超过 100 MiB，请先裁剪或压缩后重新选择。");
  }

  const video = document.createElement("video");
  const canvas = document.createElement("canvas");
  let objectUrl: string | undefined;

  try {
    video.muted = true;
    video.playsInline = true;
    video.preload = "auto";
    objectUrl = URL.createObjectURL(file);
    const source = objectUrl;

    await waitForVideo(
      video,
      signal,
      `无法读取视频信息。${CONVERSION_HINT}`,
      () => video.readyState >= video.HAVE_METADATA,
      () => {
        video.src = source;
        video.load();
      },
    );

    let duration = video.duration;
    if (!Number.isFinite(duration) && extension === "webm") {
      // 部分 WebM 缺少 Duration；远端 seek 可促使浏览器读取尾部，但不能用 currentTime 猜时长。
      await waitForVideo(
        video,
        signal,
        "无法确定此 WebM 的有限时长，请转换为带完整时长信息的 MP4（H.264 视频编码）后重试。",
        () => Number.isFinite(video.duration) && video.duration > 0 && !video.seeking,
        () => { video.currentTime = Number.MAX_SAFE_INTEGER; },
      );
      duration = video.duration;
    }
    if (!Number.isFinite(duration) || duration <= 0) {
      throw new Error("视频缺少有效的有限时长，请转换为带完整时长信息的 MP4（H.264 视频编码）后重试。");
    }

    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("当前浏览器无法创建画布，请更新浏览器或 WebView2 运行时后重试。");
    }
    const count = Math.min(MAX_VIDEO_FRAMES, Math.max(1, Math.ceil(duration)));
    const frames: VideoFrame[] = [];

    for (let index = 0; index < count; index++) {
      throwIfAborted(signal);
      const timestamp = duration * ((index + 0.5) / count);
      const failureMessage = `无法解码第 ${index + 1}/${count} 张视频画面。${CONVERSION_HINT}`;
      await waitForVideo(
        video,
        signal,
        failureMessage,
        // loadedmetadata 仅保证元数据可读；必须等 seek 完成且当前画面已有数据。
        () => !video.seeking
          && video.readyState >= video.HAVE_CURRENT_DATA
          && Math.abs(video.currentTime - timestamp) <= 0.1,
        () => { video.currentTime = timestamp; },
      );
      throwIfAborted(signal);

      const width = video.videoWidth;
      const height = video.videoHeight;
      if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
        throw new Error(`第 ${index + 1}/${count} 张画面没有有效的视频尺寸，请选择包含可解码视频轨道的文件。`);
      }
      const scale = Math.min(1, MAX_FRAME_EDGE / Math.max(width, height));
      canvas.width = Math.max(1, Math.round(width * scale));
      canvas.height = Math.max(1, Math.round(height * scale));

      let encoded: string;
      try {
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        encoded = canvas.toDataURL("image/jpeg", 0.8);
      } catch (error) {
        throw new Error(`第 ${index + 1}/${count} 张画面提取或编码失败。${CONVERSION_HINT}`, { cause: error });
      }
      throwIfAborted(signal);

      const prefix = "data:image/jpeg;base64,";
      if (!encoded.startsWith(prefix)) {
        throw new Error(`第 ${index + 1}/${count} 张画面未能编码为 JPEG，请更新浏览器或 WebView2 后重试。`);
      }
      const data = encoded.slice(prefix.length);
      const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
      const byteLength = Math.floor(data.length * 3 / 4) - padding;
      if (!data || byteLength <= 0) {
        throw new Error(`第 ${index + 1}/${count} 张画面编码为空。${CONVERSION_HINT}`);
      }
      if (byteLength > MAX_FRAME_BYTES) {
        throw new Error(`第 ${index + 1}/${count} 张画面超过图片的 10 MiB 限额，请降低视频分辨率后重试。`);
      }
      frames.push({ timestamp, data, mimeType: "image/jpeg" });
    }

    throwIfAborted(signal);
    return { duration, frames };
  } finally {
    video.removeAttribute("src");
    try {
      video.load();
    } finally {
      if (objectUrl !== undefined) URL.revokeObjectURL(objectUrl);
      canvas.width = 0;
      canvas.height = 0;
    }
  }
}
