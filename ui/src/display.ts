import type { CommentItem, LevelInfo } from './types.generated'

// 本模块只放**纯取值/分档**逻辑，不引入运行时依赖（含文案模块）：
// 这样单测能直接用 `node --experimental-strip-types` 跑，无需打包步骤。
// 文案与类名拼接由调用方按状态映射，见 index.tsx。

/** 推荐状态。楼中楼官方不给可信值，归档里留空，单独成一档。 */
export type RecommendState = 'yes' | 'no' | 'unknown'

export const recommendState = (item: CommentItem): RecommendState =>
  item.is_recommend === null ? 'unknown' : item.is_recommend ? 'yes' : 'no'

export const recommendClass = (state: RecommendState) => `cc-flag cc-flag--${state}`

/** Unix 秒 → 本地化字符串；时间戳异常时显示占位符。 */
export const formatTime = (seconds: number) =>
  seconds > 0 ? new Date(seconds * 1000).toLocaleString('zh-CN') : '—'

/** IPC 错误载荷优先取 message，其余落到调用方给的兜底文案。 */
export const errorMessage = (error: unknown, fallback: string) =>
  error && typeof error === 'object' && 'message' in error ? String(error.message) : fallback

/** 关卡 ID 只接受纯数字（与后端校验一致），空串表示未填写。 */
export const validLevelId = (value: string) => /^\d{1,20}$/.test(value.trim())

/** 热度分可能是纯数字，也可能带「万」。 */
export function parseHot(raw: string): number {
  const text = String(raw ?? '').trim()
  if (!text) return 0
  if (text.includes('万')) return (Number.parseFloat(text.replace('万', '')) || 0) * 10000 // i18n-allow: 官方数据里的「万」，非界面文案
  return Number.parseFloat(text) || 0
}

/** 好评率文案形如 `96.7%`。 */
export const parseRate = (raw: string) =>
  Number.parseFloat(String(raw ?? '').replace('%', '')) || 0

/** 热度分档：≥10 万钻石、≥1 万金色，其余默认色。阈值与旧版一致。 */
export type HotLevel = 'normal' | 'gold' | 'diamond'

export function hotLevel(raw: string): HotLevel {
  const value = parseHot(raw)
  if (value >= 100_000) return 'diamond'
  if (value >= 10_000) return 'gold'
  return 'normal'
}

/** 好评率分档：≥90 优秀、≥70 良好、≥50 一般，其余偏低。阈值与旧版一致。 */
export type RateLevel = 'low' | 'medium' | 'good' | 'excellent'

export function rateLevel(raw: string): RateLevel {
  const value = parseRate(raw)
  if (value >= 90) return 'excellent'
  if (value >= 70) return 'good'
  if (value >= 50) return 'medium'
  return 'low'
}

/** 数值文本的分档类；流光渐变只作用于文字。 */
export const hotValueClass = (level: HotLevel) =>
  level === 'normal' ? 'cc-hot' : `cc-hot cc-hot--${level}`

export const rateValueClass = (level: RateLevel) => `cc-rate cc-rate--${level}`

/**
 * 图标的分档类：只换颜色。
 *
 * 不能复用数值那套：流光靠 `-webkit-text-fill-color: transparent` + `background-clip: text` 实现，
 * 套到描边图标上会让它整只消失。
 */
export const hotIconClass = (level: HotLevel) =>
  level === 'normal' ? undefined : `cc-hot-icon--${level}`

export const rateIconClass = (level: RateLevel) => `cc-rate-icon--${level}`

/** 封面在前、去重、去掉空值——官方图集里可能混有空 URL。 */
export function gallery(level: LevelInfo): string[] {
  const all = [level.cover_url, ...level.images].map((url) => (url ?? '').trim()).filter(Boolean)
  return [...new Set(all)]
}
