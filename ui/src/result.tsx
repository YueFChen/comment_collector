import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type {
  CollectProgress,
  CommentArchive,
  CommentItem,
  ExportFormat,
} from './types.generated'
import {
  ArrowLeft,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Flame,
  FolderOpen,
  Gamepad2,
  Inbox,
  RotateCw,
  Search,
  ThumbsUp,
  Users,
} from 'lucide-react'

import type { CommentsApi } from './api'
import {
  errorMessage,
  formatTime,
  gallery,
  hotIconClass,
  hotLevel,
  hotValueClass,
  rateIconClass,
  rateLevel,
  rateValueClass,
  recommendClass,
  recommendState,
  validLevelId,
  type RecommendState,
} from './display'
import { t } from './i18n'
import { CollectProgressBar } from './progress'
import {
  clampPage,
  filterGroups,
  groupComments,
  pageSlice,
  sortGroups,
  type Filter,
  type Sort,
} from './view'

export interface ResultProps {
  api: CommentsApi
  /** 要展示的关卡；来自路由参数。 */
  levelId: string
  /** 返回采集入口。 */
  onBack: () => void
}

const FILTERS: { value: Filter; label: string }[] = [
  { value: 'all', label: t('result.filterAll') },
  { value: 'recommend', label: t('result.filterRecommend') },
  { value: 'notRecommend', label: t('result.filterNotRecommend') },
  { value: 'owner', label: t('result.filterOwner') },
]

const SORTS: { value: Sort; label: string }[] = [
  { value: 'default', label: t('result.sortDefault') },
  { value: 'newest', label: t('result.sortNewest') },
  { value: 'oldest', label: t('result.sortOldest') },
  { value: 'likes', label: t('result.sortLikes') },
  { value: 'floor', label: t('result.sortFloor') },
]

const PAGE_SIZES = [10, 20, 50, 100]

const RECOMMEND_LABEL: Record<RecommendState, string> = {
  yes: t('result.filterRecommend'),
  no: t('result.filterNotRecommend'),
  unknown: '—',
}

/**
 * 评论结果页。
 *
 * 只读本地归档并做呈现：筛选、排序、分页都只影响视图，不触发采集。
 * 唯一的写入操作是「重新采集」，且结果同样落到归档。
 */
export function CommentsResultPage({ api, levelId, onBack }: ResultProps) {
  const [archive, setArchive] = useState<CommentArchive | null>(null)
  const [directory, setDirectory] = useState('')
  const [progress, setProgress] = useState<CollectProgress | null>(null)
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  // 存导出路径（数据）而不是取好的文案：文案在渲染时才解析。
  const [exportedPath, setExportedPath] = useState<string | null>(null)
  // 视图状态：换了关卡就全部回到起点。
  const [filter, setFilter] = useState<Filter>('all')
  const [sort, setSort] = useState<Sort>('default')
  const [keyword, setKeyword] = useState('')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const [descOpen, setDescOpen] = useState(false)
  const [imageIndex, setImageIndex] = useState(0)
  const generation = useRef(0)
  const mounted = useRef(true)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      generation.current++
    }
  }, [])

  useEffect(() => {
    void api
      .exportDir()
      .then((path) => {
        if (mounted.current) setDirectory(path)
      })
      .catch(() => {
        if (mounted.current) setDirectory('')
      })
  }, [api])

  const load = useCallback(async () => {
    const request = ++generation.current
    // 换关卡时先清空视图状态，免得旧筛选条件落到新数据上。
    setFilter('all')
    setSort('default')
    setKeyword('')
    setPage(1)
    setExpanded({})
    setDescOpen(false)
    setImageIndex(0)
    setExportedPath(null)
    if (!validLevelId(levelId)) {
      setArchive(null)
      setError(t('common.invalidId'))
      setLoading(false)
      return
    }
    setLoading(true)
    setError('')
    try {
      const loaded = await api.archive(levelId)
      if (generation.current !== request || !mounted.current) return
      setArchive(loaded)
      if (!loaded) setError(t('result.noArchive'))
    } catch (cause) {
      if (generation.current === request && mounted.current) {
        setArchive(null)
        setError(errorMessage(cause, t('common.failed')))
      }
    } finally {
      if (generation.current === request && mounted.current) setLoading(false)
    }
  }, [api, levelId])

  useEffect(() => {
    void load()
  }, [load])

  const recollect = useCallback(async () => {
    const request = ++generation.current
    setBusy(true)
    setError('')
    setExportedPath(null)
    setProgress({ page: 0, fetched: 0 })
    try {
      // 一律抓全量：这个页面不提供限页入口，重新采集就是要把归档补到最新最全。
      const result = await api.collect({ level_id: levelId }, (next) => {
        if (generation.current === request && mounted.current) setProgress(next)
      })
      if (generation.current !== request || !mounted.current) return
      setArchive(result)
    } catch (cause) {
      if (generation.current === request && mounted.current) {
        setError(errorMessage(cause, t('common.failed')))
        // 失败与取消时后端也会把已抓到的页落盘（见 collect 的检查点）。
        // 只重读归档，不走 load()：那会把用户当前的筛选、排序与展开状态一起冲掉。
        const partial = await api.archive(levelId).catch(() => null)
        if (generation.current === request && mounted.current && partial) setArchive(partial)
      }
    } finally {
      if (generation.current === request && mounted.current) {
        setBusy(false)
        setProgress(null)
      }
    }
  }, [api, levelId])

  const stop = useCallback(() => {
    void api.cancel().catch(() => undefined)
  }, [api])

  /** 打开导出目录；具体路径由宿主从插件取，前端只表达「打开」这个意图。 */
  const reveal = useCallback(async () => {
    setError('')
    try {
      await api.revealDir()
    } catch (cause) {
      if (mounted.current) setError(errorMessage(cause, t('common.failed')))
    }
  }, [api])

  const download = useCallback(
    async (format: ExportFormat) => {
      setError('')
      setExportedPath(null)
      try {
        const outcome = await api.export(levelId, format)
        if (mounted.current) setExportedPath(outcome.path)
      } catch (cause) {
        if (mounted.current) setError(errorMessage(cause, t('common.failed')))
      }
    },
    [api, levelId],
  )

  const groups = useMemo(() => groupComments(archive?.comments ?? []), [archive])
  const visible = useMemo(
    () => sortGroups(filterGroups(groups, filter, keyword), sort),
    [groups, filter, keyword, sort],
  )
  const safePage = clampPage(page, visible.length, pageSize)
  const pageCount = Math.max(1, Math.ceil(visible.length / pageSize))
  const shown = useMemo(() => pageSlice(visible, safePage, pageSize), [visible, safePage, pageSize])

  const images = useMemo(() => (archive ? gallery(archive.level) : []), [archive])
  const coverIndex = images.length ? Math.min(imageIndex, images.length - 1) : 0
  const cover = images[coverIndex] ?? ''
  // 分档与数值文本的类分开取：流光只能套文字，套到图标上会让它消失。
  const hot = hotLevel(archive?.level.hot_score ?? '')
  const rate = rateLevel(archive?.level.good_rate ?? '')
  // 展开控件只针对当前页：分页之后「全部展开」若跨页生效，用户看不到发生了什么。
  const expandable = useMemo(() => shown.filter((item) => item.subs.length > 0), [shown])
  const expandedOnPage = expandable.filter((item) => expanded[item.main.reply_id]).length

  const turnImage = (delta: number) => {
    if (images.length < 2) return
    setImageIndex((current) => (current + delta + images.length) % images.length)
  }

  const toggleReplies = (id: string) =>
    setExpanded((current) => ({ ...current, [id]: !current[id] }))

  const setAllExpanded = (value: boolean) =>
    setExpanded((current) => ({
      ...current,
      ...Object.fromEntries(expandable.map((item) => [item.main.reply_id, value])),
    }))

  /** 任何筛选条件变化都回到第一页，避免停在空白页上。 */
  const changeFilter = (next: Filter) => {
    setFilter(next)
    setPage(1)
  }
  const changeSort = (next: Sort) => {
    setSort(next)
    setPage(1)
  }
  const changeKeyword = (next: string) => {
    setKeyword(next)
    setPage(1)
  }
  const changePageSize = (next: number) => {
    setPageSize(next)
    setPage(1)
  }
  const clearFilters = () => {
    setFilter('all')
    setKeyword('')
    setPage(1)
  }

  const filtering = filter !== 'all' || keyword.trim() !== ''
  const canExport = Boolean(archive) && !busy

  const renderComment = (item: CommentItem, isSub: boolean) => {
    const state = recommendState(item)
    return (
      <article className={isSub ? 'cc-comment cc-comment--sub' : 'cc-comment'} key={item.reply_id}>
        {item.avatar_url ? (
          <img className="cc-avatar" src={item.avatar_url} alt="" loading="lazy" />
        ) : (
          <span className="cc-avatar cc-avatar--letter">{item.nickname.slice(0, 1) || '?'}</span>
        )}
        <div className="cc-body">
          <div className="cc-meta">
            <span className="cc-name">{item.nickname || t('result.anonymous')}</span>
            <span className="cc-uid">UID {item.uid || '—'}</span>
            {item.is_owner && <span className="cc-badge">{t('result.isOwner')}</span>}
            {!isSub && <span className={recommendClass(state)}>{RECOMMEND_LABEL[state]}</span>}
            {isSub && item.reply_to && <span className="cc-reply-to">{t('result.replyTo', { name: item.reply_to })}</span>}
            {item.ip_region && <span className="cc-ip">{item.ip_region}</span>}
            <time className="cc-time">{formatTime(item.created_at)}</time>
          </div>
          {/* 不推荐的内容带左侧红边，与旧版一致：扫一眼就能看出负面评价。 */}
          <p className={state === 'no' && !isSub ? 'cc-content cc-content--negative' : 'cc-content'}>
            {item.content || '—'}
          </p>
          <div className="cc-foot">
            {!isSub && <span>{item.floor_id ? t('result.floor', { floor: item.floor_id }) : '—'}</span>}
            <span>{t('result.likeCount', { count: item.like_count })}</span>
            {!isSub && <span>{t('result.replyCount', { count: item.reply_count })}</span>}
          </div>
        </div>
      </article>
    )
  }

  return (
    <section className="cc" aria-label={t('result.title')}>
      <div className="cc-result-actions">
        <button className="cc-back" onClick={onBack}>
          <ArrowLeft aria-hidden />
          {t('common.back')}
        </button>
        <p className="cc-result-meta">
          {t('common.levelId')} {levelId}
          {archive
            ? ` · ${t('result.updatedAt')} ${formatTime(archive.updated_at)} · ${t('result.fetchCount', { count: archive.fetch_count })}`
            : ''}
        </p>
        <button
          className="cc-primary"
          disabled={busy || loading || !validLevelId(levelId)}
          onClick={() => void recollect()}
        >
          <RotateCw className={busy ? 'cc-spin' : undefined} aria-hidden />
          {t('result.recollect')}
        </button>
      </div>

      {error && (
        <div className="cc-error" role="alert">
          <span>{error}</span>
        </div>
      )}
      {exportedPath && (
        <div className="cc-notice" role="status">
          <span>{t('result.exported', { path: exportedPath })}</span>
        </div>
      )}
      {busy && <CollectProgressBar progress={progress} onCancel={stop} />}

      {loading ? (
        <div className="cc-empty" role="status">
          {t('result.loading')}
        </div>
      ) : !archive ? (
        <div className="cc-empty">
          <Inbox aria-hidden />
          <p>{t('result.noArchive')}</p>
          <button className="cc-link" onClick={onBack}>
            {t('result.goCollect')}
          </button>
        </div>
      ) : (
        <>
          <article className="cc-level">
            {cover && (
              <div className="cc-cover">
                <img src={cover} alt="" loading="lazy" />
                {images.length > 1 && (
                  <>
                    <button
                      className="cc-cover-nav cc-cover-nav--prev"
                      aria-label={t('result.imagePrev')}
                      onClick={() => turnImage(-1)}
                    >
                      <ChevronLeft aria-hidden />
                    </button>
                    <button
                      className="cc-cover-nav cc-cover-nav--next"
                      aria-label={t('result.imageNext')}
                      onClick={() => turnImage(1)}
                    >
                      <ChevronRight aria-hidden />
                    </button>
                    <span className="cc-cover-index">
                      {t('result.imageIndex', { index: coverIndex + 1, total: images.length })}
                    </span>
                  </>
                )}
              </div>
            )}
            <div className="cc-level-body">
              <h2>{archive.level.level_name || t('common.unknownLevel')}</h2>
              {archive.level.desc && (
                <div className={descOpen ? 'cc-desc cc-desc--open' : 'cc-desc'}>
                  <p>{archive.level.desc}</p>
                  <button className="cc-desc-toggle" onClick={() => setDescOpen((open) => !open)}>
                    {descOpen ? t('result.collapseDesc') : t('result.expandDesc')}
                  </button>
                </div>
              )}
              <div className="cc-tiles">
                <div className="cc-tile">
                  <Flame className={hotIconClass(hot)} aria-hidden />
                  <div>
                    <small>{t('result.hotScore')}</small>
                    <strong className={hotValueClass(hot)}>{archive.level.hot_score || '—'}</strong>
                  </div>
                </div>
                <div className="cc-tile">
                  <ThumbsUp className={rateIconClass(rate)} aria-hidden />
                  <div>
                    <small>{t('result.goodRate')}</small>
                    <strong className={rateValueClass(rate)}>{archive.level.good_rate || '—'}</strong>
                  </div>
                </div>
                <div className="cc-tile">
                  <Users aria-hidden />
                  <div>
                    <small>{t('result.playRange')}</small>
                    <strong>{archive.level.play_range || '—'}</strong>
                  </div>
                </div>
                <div className="cc-tile">
                  <Gamepad2 aria-hidden />
                  <div>
                    <small>{t('result.playType')}</small>
                    <strong>{archive.level.play_type || '—'}</strong>
                  </div>
                </div>
              </div>
            </div>
          </article>

          <div className="cc-actions">
            <nav className="cc-filters" aria-label={t('result.filterAria')}>
              {FILTERS.map((item) => (
                <button
                  key={item.value}
                  aria-pressed={filter === item.value}
                  onClick={() => changeFilter(item.value)}
                >
                  {item.label}
                </button>
              ))}
            </nav>
            <label>
              {t('result.sortLabel')}
              <select
                aria-label={t('result.sortLabel')}
                value={sort}
                onChange={(event) => changeSort(event.target.value as Sort)}
              >
                {SORTS.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="cc-search">
              <Search aria-hidden />
              <input
                value={keyword}
                placeholder={t('result.searchPlaceholder')}
                onChange={(event) => changeKeyword(event.target.value)}
              />
            </label>
            {expandedOnPage < expandable.length && (
              <button className="cc-ghost cc-expand-toggle" onClick={() => setAllExpanded(true)}>
                {t('result.expandAll')}
              </button>
            )}
            {expandedOnPage > 0 && (
              <button className="cc-ghost cc-expand-toggle" onClick={() => setAllExpanded(false)}>
                {t('result.collapseAll')}
              </button>
            )}
          </div>

          {/* 导出与「当前区间」同处一行：导出按钮本来就不需要单独占一条。 */}
          <div className="cc-result-bar">
            <span className="cc-range">
              {visible.length === 0
                ? t('result.pageRange', { from: 0, to: 0, total: 0 })
                : t('result.pageRange', {
                    from: (safePage - 1) * pageSize + 1,
                    to: Math.min(safePage * pageSize, visible.length),
                    total: visible.length,
                  })}
            </span>
            {directory && (
              <button className="cc-dir" title={t('result.openDir')} onClick={() => void reveal()}>
                <FolderOpen aria-hidden />
                <span className="cc-dir-text">
                  {t('result.exportDir')}<code>{directory}</code>
                </span>
              </button>
            )}
            <div className="cc-export-actions">
              <button disabled={!canExport} onClick={() => void download('csv')}>
                {t('result.exportCsv')}
              </button>
              <button disabled={!canExport} onClick={() => void download('excel')}>
                {t('result.exportExcel')}
              </button>
              <button disabled={!canExport} onClick={() => void download('json')}>
                {t('result.exportJson')}
              </button>
            </div>
          </div>

          {visible.length === 0 ? (
            <div className="cc-empty">
              <Search aria-hidden />
              {/* 归档本身为空与「筛没了」是两回事，提示要分开。 */}
              <p>{groups.length === 0 ? t('result.noComments') : t('result.noMatch')}</p>
              {filtering && groups.length > 0 && (
                <button className="cc-link" onClick={clearFilters}>
                  {t('result.clearFilters')}
                </button>
              )}
            </div>
          ) : (
            <>
              <section className="cc-list">
                {shown.map((item) => {
                  const isOpen = Boolean(expanded[item.main.reply_id])
                  return (
                    <div className="cc-group" key={item.main.reply_id}>
                      {renderComment(item.main, false)}
                      {item.subs.length > 0 && (
                        <button
                          className="cc-replies-toggle"
                          aria-expanded={isOpen}
                          onClick={() => toggleReplies(item.main.reply_id)}
                        >
                          <ChevronDown
                            className={isOpen ? 'cc-chevron cc-chevron--open' : 'cc-chevron'}
                            aria-hidden
                          />
                          {isOpen ? t('result.collapseReplies') : t('result.expandReplies')}
                        </button>
                      )}
                      {isOpen && item.subs.map((sub) => renderComment(sub, true))}
                    </div>
                  )
                })}
              </section>

              {pageCount > 1 && (
                <nav className="cc-pagination" aria-label={t('result.pageSizeLabel')}>
                  <label>
                    <select
                      aria-label={t('result.pageSizeLabel')}
                      value={pageSize}
                      onChange={(event) => changePageSize(Number(event.target.value))}
                    >
                      {PAGE_SIZES.map((size) => (
                        <option key={size} value={size}>
                          {size} {t('result.pageSize')}
                        </option>
                      ))}
                    </select>
                  </label>
                  <div className="cc-pager">
                    <button
                      aria-label={t('result.prevPage')}
                      disabled={safePage <= 1}
                      onClick={() => setPage(safePage - 1)}
                    >
                      <ChevronLeft aria-hidden />
                    </button>
                    <span>{t('result.pageOf', { page: safePage, pages: pageCount })}</span>
                    <button
                      aria-label={t('result.nextPage')}
                      disabled={safePage >= pageCount}
                      onClick={() => setPage(safePage + 1)}
                    >
                      <ChevronRight aria-hidden />
                    </button>
                  </div>
                </nav>
              )}
            </>
          )}
        </>
      )}
    </section>
  )
}
