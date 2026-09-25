import type { CommentItem } from './types.generated'

/** 主评论 + 它的楼中楼；排序与分页都以「组」为单位，楼中楼不会被拆散。 */
export interface Group {
  main: CommentItem
  subs: CommentItem[]
}

export type Filter = 'all' | 'recommend' | 'notRecommend' | 'owner'

/**
 * 排序方式。
 *
 * `default` 保留官方热度排序的原始顺序——那本身也是有用的信息。
 */
export type Sort = 'default' | 'newest' | 'oldest' | 'likes' | 'floor'

/**
 * 把扁平列表还原成「主评论 + 楼中楼」分组。
 *
 * 按 `parent_id` 归组而不是假设相邻：归档合并后楼中楼不一定紧跟其主评论。
 */
export function groupComments(comments: CommentItem[]): Group[] {
  const order: string[] = []
  const buckets = new Map<string, { main: CommentItem | null; subs: CommentItem[] }>()
  for (const item of comments) {
    const key = item.is_sub ? item.parent_id : item.reply_id
    let bucket = buckets.get(key)
    if (!bucket) {
      bucket = { main: null, subs: [] }
      buckets.set(key, bucket)
      order.push(key)
    }
    if (item.is_sub) bucket.subs.push(item)
    else bucket.main = item
  }
  const out: Group[] = []
  for (const key of order) {
    const bucket = buckets.get(key)
    // 官方数据异常时可能出现没有主评论的空壳，丢掉而不是渲染无主的楼中楼。
    if (bucket?.main) out.push({ main: bucket.main, subs: bucket.subs })
  }
  return out
}

const matches = (item: CommentItem, keyword: string) =>
  item.nickname.toLowerCase().includes(keyword) ||
  item.uid.includes(keyword) ||
  item.ip_region.toLowerCase().includes(keyword) ||
  item.content.toLowerCase().includes(keyword)

/** 按筛选条件与关键词过滤；关键词命中任一楼中楼时整组保留，便于看到上下文。 */
export function filterGroups(groups: Group[], filter: Filter, keyword: string): Group[] {
  const needle = keyword.trim().toLowerCase()
  return groups.filter((group) => {
    if (filter === 'recommend' && group.main.is_recommend !== true) return false
    if (filter === 'notRecommend' && group.main.is_recommend !== false) return false
    if (filter === 'owner' && !group.main.is_owner) return false
    if (!needle) return true
    return matches(group.main, needle) || group.subs.some((sub) => matches(sub, needle))
  })
}

/** 楼层排序键：缺失或非数字（官方异常）排在最后，而不是当成 0 楼挤到最前。
 *  用有限大数而非 Infinity：两个缺失楼层相减时 Infinity - Infinity 是 NaN，
 *  比较函数一旦返回 NaN，排序结果就不可预期。 */
const floorRank = (floor: string): number => {
  const value = Number.parseInt(floor, 10)
  return Number.isFinite(value) ? value : Number.MAX_SAFE_INTEGER
}

/** 排序：始终返回新数组，不改动入参。 */
export function sortGroups(groups: Group[], sort: Sort): Group[] {
  if (sort === 'default') return groups.slice()
  return groups.slice().sort((left, right) => {
    switch (sort) {
      case 'newest':
        return right.main.created_at - left.main.created_at
      case 'oldest':
        return left.main.created_at - right.main.created_at
      case 'likes':
        return right.main.like_count - left.main.like_count
      case 'floor':
        return floorRank(left.main.floor_id) - floorRank(right.main.floor_id)
    }
  })
}

/** 把页码夹到有效范围内；筛掉一批结果后页码不会停在空白页上。 */
export function clampPage(page: number, total: number, pageSize: number): number {
  const pages = Math.max(1, Math.ceil(total / pageSize))
  return Math.min(Math.max(1, page), pages)
}

/** 当前页的分组，以及用于展示的区间文案数字。 */
export function pageSlice(groups: Group[], page: number, pageSize: number): Group[] {
  const start = (page - 1) * pageSize
  return groups.slice(start, start + pageSize)
}
