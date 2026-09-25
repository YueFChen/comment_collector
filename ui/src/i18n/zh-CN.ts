/**
 * 评论采集器的界面文案（B7）。
 *
 * 键 = `<域>.<语义>`，域取页面 / 视图名（entry / result / progress / common），段内 camelCase；
 * 多处复用的归 `common.*`。约定见 docs/知识库早期调研与评测.md §8.1。
 */
export const zh = {
  // 入口页
  'entry.title': '评论采集器',
  'entry.startCollect': '开始采集',
  'entry.recent': '最近采集',
  'entry.recentEmpty': '暂无采集记录，采集一次后这里会列出已留存的关卡。',
  'entry.levelIdPlaceholder': '输入关卡 ID（纯数字）',
  'entry.historyCount': '{count} 条',

  // 结果页
  'result.title': '评论采集结果',
  'result.goCollect': '去采集',
  'result.recollect': '重新采集',
  'result.loading': '正在读取本地归档…',
  'result.noArchive': '本地还没有这个关卡的归档。',
  'result.updatedAt': '更新于',
  'result.fetchCount': '累计采集 {count} 次',
  'result.hotScore': '热度',
  'result.goodRate': '好评率',
  'result.playRange': '人数',
  'result.playType': '类型',
  'result.expandDesc': '展开',
  'result.collapseDesc': '收起',
  'result.imagePrev': '上一张',
  'result.imageNext': '下一张',
  'result.imageIndex': '{index}/{total}',
  'result.filterAria': '评论筛选',
  'result.filterAll': '全部',
  'result.filterRecommend': '推荐',
  'result.filterNotRecommend': '不推荐',
  'result.filterOwner': '只看作者',
  'result.sortLabel': '排序',
  'result.sortDefault': '默认（热度）',
  'result.sortNewest': '最新优先',
  'result.sortOldest': '最早优先',
  'result.sortLikes': '点赞最多',
  'result.sortFloor': '楼层正序',
  'result.searchPlaceholder': '搜索昵称、UID、属地或内容',
  'result.clearFilters': '清除筛选',
  'result.noComments': '该关卡还没有评论。',
  'result.noMatch': '没有符合条件的评论。',
  'result.expandAll': '展开全部回复',
  'result.collapseAll': '收起全部回复',
  'result.floor': '{floor} 楼',
  'result.replyTo': '回复 {name}',
  'result.isOwner': '作者',
  'result.likeCount': '{count} 赞',
  'result.replyCount': '{count} 条回复',
  'result.expandReplies': '展开回复',
  'result.collapseReplies': '收起回复',
  'result.pageSize': '条/页',
  'result.pageSizeLabel': '每页条数',
  'result.pageRange': '第 {from}–{to} 条 / 共 {total} 条主评论',
  'result.prevPage': '上一页',
  'result.nextPage': '下一页',
  'result.pageOf': '{page}/{pages}',
  'result.exportCsv': '导出 CSV',
  'result.exportExcel': '导出 Excel',
  'result.exportJson': '导出 JSON',
  'result.exportDir': '导出目录：',
  /** 点「导出目录」时在系统文件管理器里打开它。 */
  'result.openDir': '打开导出目录',
  'result.exported': '已导出：{path}',
  'result.anonymous': '匿名用户',

  // 抓取进度（入口页与结果页共用同一个进度条组件）
  'progress.collecting': '正在抓取评论…',
  'progress.fetched': '已抓取 {page} 页 · {fetched} 条',

  // 跨文件复用
  'common.back': '返回',
  'common.levelId': '关卡 ID',
  'common.cancel': '取消',
  'common.invalidId': '关卡 ID 应为纯数字。',
  'common.failed': '操作失败',
  'common.unknownLevel': '未命名关卡',
} as const
