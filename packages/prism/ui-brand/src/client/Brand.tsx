/**
 * 棱镜的标志和名称（原仓库 docs/ui-design.md 第 0 章）：圆角方块里一块三棱镜，右侧射出三道光。
 * 浅色主题黑底白线、深色主题浅底黑线，颜色取主题的主文字色和它的反色。
 */
import type { CSSProperties } from 'react'
import type { SidebarBrandMarkOwnerProps } from '@deepseek-ai/dsh-client-ui-sidebar/client'
import type { HeroBrandMarkOwnerProps } from '@deepseek-ai/dsh-client-ui-conversation/client'

/** 方块圆角占边长的比例，和应用图标一致。 */
const BADGE_RADIUS_RATIO = 0.27
/** 线稿占方块边长的比例。 */
const MARK_SIZE_RATIO = 0.65

/**
 * 圆角方块 + 三棱镜线稿。
 * @param props - 边长和可选的外层 class。
 * @returns 棱镜标志。
 */
export function PrismBadge({ size, className }: { size: number; className?: string | undefined }) {
  const mark = Math.round(size * MARK_SIZE_RATIO)
  const badge: CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexShrink: 0,
    width: size,
    height: size,
    borderRadius: size * BADGE_RADIUS_RATIO,
    background: 'var(--dsw-alias-label-primary)',
    color: 'var(--dsw-alias-label-primary-inverted)',
  }
  return (
    <span className={className} style={badge} aria-hidden="true">
      <svg
        width={mark}
        height={mark}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.9}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M9 3.5L2.5 19.5h13z" />
        <path d="M12.5 11.5L21.5 7M12.5 11.5h9M12.5 11.5L21.5 16" />
      </svg>
    </span>
  )
}

/**
 * 侧栏标志。
 * @param props - 侧栏要求的边长。
 * @returns 棱镜标志。
 */
export function SidebarMark({ size }: SidebarBrandMarkOwnerProps) {
  return <PrismBadge size={size} />
}

/**
 * 新会话首屏标题前的标志。
 * @param props - 首屏要求的边长和保持布局的 class。
 * @returns 棱镜标志。
 */
export function HeroMark({ size, className }: HeroBrandMarkOwnerProps) {
  return <PrismBadge size={size} className={className} />
}

/**
 * 侧栏名称："棱镜"，后面小字写英文名和构建版本，方便认出是哪个构建。
 * @returns 名称文字。
 */
export function SidebarName() {
  const version = process.env.DSH_CLIENT_VERSION
  return (
    <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 6, whiteSpace: 'nowrap' }}>
      <span style={{ fontSize: 17, fontWeight: 600 }}>棱镜</span>
      <span style={{ fontSize: 12, color: 'var(--dsw-alias-label-tertiary)' }}>
        Prism{version === undefined ? '' : ` ${version}`}
      </span>
    </span>
  )
}
