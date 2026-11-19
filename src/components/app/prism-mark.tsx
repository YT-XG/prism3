/**
 * 品牌标记：一个极简三棱镜 + 一束被折射的光。
 *
 * 只用 `currentColor` 画轮廓、用 `var(--brand)` 画光束，因此三轴主题 / 强调色变化时
 * 标记会自动跟着走。主窗口标题栏与 hub 顶栏共用同一枚，保证两个窗口是同一个产品。
 */
export function PrismMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      className={className}
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M12 3.6 21 19.6H3L12 3.6Z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path d="M4.2 12h6.6" stroke="var(--brand)" strokeWidth="1.5" strokeLinecap="round" />
      <path
        d="m11.4 12 6.8-3.3M11.4 12h7.2M11.4 12l6.8 3.3"
        stroke="var(--brand)"
        strokeWidth="1.2"
        strokeLinecap="round"
        opacity="0.65"
      />
    </svg>
  );
}
