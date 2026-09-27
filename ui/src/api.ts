import type {
  ArchiveSummary,
  ArchiveOverview,
  ArchiveViewPage,
  ArchiveViewQuery,
  CollectProgress,
  CommentQuery,
  ExportFormat,
  ExportOutcome,
  FavoriteLevel,
} from './types.generated'

/** 插件前端与宿主之间的契约；实现由 shell 提供（见 apps/desktop）。 */
export interface CommentsApi {
  /** 采集并并入本地归档；每翻完一页回调一次进度。 */
  collect(query: CommentQuery, onProgress: (progress: CollectProgress) => void): Promise<ArchiveOverview>
  /** 请正在进行的采集停下；已抓到的页仍会留在归档里，只是不再继续翻。 */
  cancel(): Promise<void>
  /** 按筛选、排序和分页读取评论；归档不存在时为 null。 */
  archiveView(query: ArchiveViewQuery): Promise<ArchiveViewPage | null>
  /** 已采集的关卡清单。 */
  archives(): Promise<ArchiveSummary[]>
  /** 本机保存的奇域关卡收藏。 */
  favorites(): Promise<FavoriteLevel[]>
  /** 切换某关卡的本地收藏状态，返回切换后的状态。 */
  toggleFavorite(levelId: string): Promise<boolean>
  /** 导出到固定目录，返回落地路径。 */
  export(levelId: string, format: ExportFormat): Promise<ExportOutcome>
  /** 导出目录（展示给用户）。 */
  exportDir(): Promise<string>
  /** 在系统文件管理器里打开导出目录。 */
  revealDir(): Promise<void>
}
