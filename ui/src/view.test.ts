import assert from 'node:assert/strict'
import test from 'node:test'
import type { CommentItem } from './types.generated'

import { clampPage, filterGroups, groupComments, pageSlice, sortGroups } from './view.ts'

const main = (id: string, over: Partial<CommentItem> = {}): CommentItem => ({
  reply_id: id,
  floor_id: id,
  uid: `u${id}`,
  nickname: `用户${id}`,
  avatar_url: '',
  content: `内容${id}`,
  is_recommend: true,
  is_owner: false,
  like_count: 0,
  reply_count: 0,
  created_at: Number(id),
  ip_region: '浙江',
  is_sub: false,
  parent_id: '',
  reply_to: '',
  ...over,
})

const sub = (id: string, parent: string, over: Partial<CommentItem> = {}): CommentItem =>
  main(id, { is_sub: true, parent_id: parent, floor_id: '', ...over })

test('groups sub replies under their parent regardless of order', () => {
  const groups = groupComments([sub('9', '1'), main('1'), main('2'), sub('8', '1')])
  assert.equal(groups.length, 2)
  assert.equal(groups[0].main.reply_id, '1')
  assert.deepEqual(
    groups[0].subs.map((item) => item.reply_id),
    ['9', '8'],
  )
  assert.equal(groups[1].main.reply_id, '2')
})

test('drops orphan sub replies without a main comment', () => {
  const groups = groupComments([sub('9', '404'), main('1')])
  assert.equal(groups.length, 1)
  assert.equal(groups[0].main.reply_id, '1')
})

test('filters by recommend state and owner, and searches ip region', () => {
  const groups = groupComments([
    main('1', { is_recommend: true }),
    main('2', { is_recommend: false }),
    main('3', { is_recommend: null, is_owner: true }),
  ])
  assert.equal(filterGroups(groups, 'all', '').length, 3)
  assert.deepEqual(
    filterGroups(groups, 'recommend', '').map((g) => g.main.reply_id),
    ['1'],
  )
  assert.deepEqual(
    filterGroups(groups, 'notRecommend', '').map((g) => g.main.reply_id),
    ['2'],
  )
  assert.deepEqual(
    filterGroups(groups, 'owner', '').map((g) => g.main.reply_id),
    ['3'],
  )
  // 楼中楼命中关键词时整组保留，否则会看到没有上下文的回复。
  const withSub = groupComments([main('1', { content: '无关' }), sub('9', '1', { content: '关键内容' })])
  assert.equal(filterGroups(withSub, 'all', '关键').length, 1)
  assert.equal(filterGroups(groups, 'all', '浙江').length, 3)
  assert.equal(filterGroups(groups, 'all', '不存在').length, 0)
})

test('sorts by time, likes and floor without mutating input', () => {
  const groups = groupComments([
    main('2', { created_at: 200, like_count: 5, floor_id: '10' }),
    main('1', { created_at: 100, like_count: 50, floor_id: '2' }),
  ])
  const original = groups.map((g) => g.main.reply_id)
  assert.deepEqual(
    sortGroups(groups, 'newest').map((g) => g.main.reply_id),
    ['2', '1'],
  )
  assert.deepEqual(
    sortGroups(groups, 'oldest').map((g) => g.main.reply_id),
    ['1', '2'],
  )
  assert.deepEqual(
    sortGroups(groups, 'likes').map((g) => g.main.reply_id),
    ['1', '2'],
  )
  assert.deepEqual(
    sortGroups(groups, 'floor').map((g) => g.main.reply_id),
    ['1', '2'],
  )
  // default 保留原始顺序，且任何排序都不得改动入参。
  assert.deepEqual(
    sortGroups(groups, 'default').map((g) => g.main.reply_id),
    original,
  )
  assert.deepEqual(
    groups.map((g) => g.main.reply_id),
    original,
  )
})

test('floor sort puts missing floor numbers last', () => {
  const groups = groupComments([main('1', { floor_id: '' }), main('2', { floor_id: '3' })])
  assert.deepEqual(
    sortGroups(groups, 'floor').map((g) => g.main.reply_id),
    ['2', '1'],
  )
})

test('clamps page after the result set shrinks', () => {
  assert.equal(clampPage(1, 0, 20), 1)
  assert.equal(clampPage(9, 25, 20), 2)
  assert.equal(clampPage(0, 25, 20), 1)
  const groups = groupComments([main('1'), main('2'), main('3')])
  assert.deepEqual(
    pageSlice(groups, 2, 2).map((g) => g.main.reply_id),
    ['3'],
  )
})
