import { useEffect, useState } from "react";

/**
 * 退场动画的小工具：`open` 变 false 后**先别卸载**，等动画跑完再移除。
 *
 * 为什么需要它：React 直接 `{open ? <X/> : null}` 是同步卸载，元素一消失 CSS 动画根本没机会跑，
 * 于是「关开始菜单 / 关电量抽屉 / 收起搜索面板」全是硬切。这里多留 `ms` 毫秒，
 * 由调用方用 `exiting` 挂上 `*-out` 动画类。
 *
 * 注意：`ms` 要和 CSS 里那条 out 动画的时长一致，否则会看到「动画没跑完就被删」或「空等」。
 */
export function usePresence(open: boolean, ms = 140) {
  const [rendered, setRendered] = useState(open);

  useEffect(() => {
    if (open) {
      setRendered(true);
      return;
    }
    const timer = window.setTimeout(() => setRendered(false), ms);
    return () => window.clearTimeout(timer);
  }, [open, ms]);

  return { rendered, exiting: rendered && !open };
}
