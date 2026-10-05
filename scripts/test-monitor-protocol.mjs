// Local SDK protocol integration with a mock Core. No public network requests.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, mkdir, writeFile, readFile, readdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { createInterface } from 'node:readline'

const root = await mkdtemp(join(tmpdir(), 'comment-monitor-protocol-'))
const config = { enabled: true, level_ids: ['100000001'], incremental_interval_secs: 60,
  full_interval_secs: 300, max_parallel: 2, request_interval_ms: 250, overlap_pages: 2,
  incremental_max_pages: 50 }
await mkdir(join(root, 'archive-v1'))
await writeFile(join(root, 'archive-v1', 'monitor-config.json'), JSON.stringify(config))
const legacyDir = join(root, 'archive-v1', 'snapshots', '100000001')
await mkdir(legacyDir, { recursive: true })
const legacyPath = join(legacyDir, 'legacy.json')
await writeFile(legacyPath, 'preserved legacy data')
const binary = resolve(process.argv[2] ?? `target/debug/wonderland-comment-collector${process.platform === 'win32' ? '.exe' : ''}`)
const child = spawn(binary, [], { env: { ...process.env, WONDERLAND_PLUGIN_DATA_DIR: root }, stdio: ['pipe', 'pipe', 'pipe'] })
let stderr = '', serial = 0, networkRequests = 0, hello = false, failure = null
const pending = new Map()
child.stderr.on('data', (chunk) => { stderr += chunk })
const envelope = { protocol: 'wonderland-plugin', version: '1.0.0' }
const send = (frame) => child.stdin.write(JSON.stringify({ ...envelope, ...frame }) + '\n')
const lines = createInterface({ input: child.stdout })
lines.on('line', (line) => {
  try {
    const frame = JSON.parse(line)
    assert.equal(frame.protocol, envelope.protocol)
    if (frame.type === 'hello') { hello = true; return }
    if (frame.type === 'request') {
      assert.equal(frame.method, 'core.network.public')
      // The startup connection must not inherit another plugin's service context.
      assert.equal(frame.serviceContext, undefined)
      const req = frame.params
      assert(!req.headers.some(([key]) => /^(cookie|authorization)$/i.test(key)))
      networkRequests++
      let data
      if (req.method === 'GET') {
        assert(req.url.includes('/level/detail?level_id=100000001&uid='))
        data = { level_info: { level_id: '100000001', level_name: 'Protocol fixture' } }
      } else {
        const body = JSON.parse(req.body)
        assert.equal(body.uid, '')
        assert.equal(body.region, 'cn_gf01')
        assert.equal(body.cursor.sort_type, 'SORT_TYPE_HOT')
        assert.equal(body.cursor.size, 20)
        data = { reply_list: [{ reply_id: 'r1', floor_id: 1, content: 'Mock public reply',
          created_at: 1000, is_recommend: true, reply_stat: { reply_count: 0 } }], cursor: { next: '', has_more: false, sort_type: 'SORT_TYPE_HOT' } }
      }
      send({ type: 'result', id: frame.id, result: { contentBase64: Buffer.from(JSON.stringify({ retcode: 0, data })).toString('base64') } })
    } else if (frame.type === 'result' || frame.type === 'error') {
      const call = pending.get(frame.id)
      assert(call, `Unexpected response ${frame.id}`)
      pending.delete(frame.id)
      if (frame.type === 'error') call.reject(new Error(JSON.stringify(frame.error)))
      else call.resolve(frame.result)
    }
  } catch (error) { failure = error }
})
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function until(predicate, description) {
  const deadline = Date.now() + 15_000
  while (!predicate()) {
    if (failure) throw failure
    if (child.exitCode !== null) throw new Error(`Backend stopped: ${stderr}`)
    if (Date.now() > deadline) throw new Error(`Timed out: ${description}; ${stderr}`)
    await delay(20)
  }
}
function rpc(method, params = {}) {
  const id = `host-${++serial}`
  return new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); send({ type: 'request', id, method, params }) })
}
try {
  send({ type: 'hello', role: 'host', pluginId: 'comment_collector' })
  // Enabled monitoring must resume from the handshake, without a UI request.
  await until(() => hello && networkRequests >= 2, 'background startup scan')
  let status
  const deadline = Date.now() + 15_000
  do {
    status = await rpc('monitor_status')
    if (!status.levels[0].running && status.levels[0].last_full_success_at > 0) break
    assert(Date.now() < deadline, 'Background scan did not complete')
    await delay(50)
  } while (true)
  assert.equal(status.levels[0].last_mode, 'full')
  assert.equal(status.levels[0].consecutive_failures, 0)
  assert.equal(status.levels[0].next_collect_at, status.levels[0].last_attempt_at + config.incremental_interval_secs)
  const archives = await rpc('archives')
  assert.equal(archives.length, 1)
  assert.equal(archives[0].collection_state, 'complete')
  assert.deepEqual(archives[0].last_new_counts, { recommended: 1, not_recommended: 0 })
  const page = await rpc('archive_view', { level_id: '100000001', offset: 0, limit: 100,
    filter: 'all', sort: 'default', keyword: '' })
  assert.equal(page.groups[0].main.content, 'Mock public reply')
  await assert.rejects(rpc('snapshots', { level_id: '100000001', offset: 0, limit: 100 }))
  await assert.rejects(rpc('snapshot_page', { level_id: '100000001', snapshot_id: 'legacy', offset: 0, limit: 100 }))
  assert.deepEqual(await readdir(legacyDir), ['legacy.json'])
  assert.equal(await readFile(legacyPath, 'utf8'), 'preserved legacy data')
  const paused = await rpc('monitor_configure', { config: { ...config, enabled: false } })
  assert.equal(paused.enabled, false)
  assert.equal((await rpc('monitor_status')).levels[0].next_collect_at, null)
  const changed = await rpc('monitor_configure', { config: { ...config, enabled: false, incremental_interval_secs: 120 } })
  assert.equal((await rpc('monitor_config')).incremental_interval_secs, 120)
  await rpc('monitor_configure', { config: paused })
  assert.equal(changed.incremental_interval_secs, 120)
  const added = await rpc('monitor_add', { level_id: '000000002' })
  assert.deepEqual(added.level_ids, ['100000001', '000000002'])
  assert.equal(added.enabled, false, 'adding must preserve the existing pause')
  const removed = await rpc('monitor_remove', { level_id: '000000002' })
  assert.deepEqual(removed.level_ids, ['100000001'])
  assert.equal(removed.incremental_interval_secs, config.incremental_interval_secs)
  const retained = await rpc('archives')
  assert.deepEqual(retained, archives)
  const requestsAtPause = networkRequests
  await delay(1100)
  assert.equal(networkRequests, requestsAtPause)
  if (failure) throw failure
  console.log('PASS: SDK handshake, automatic startup monitoring, controlled anonymous HTTP, status, current archive stats, removed history methods, pause, atomic target add/remove')
} finally {
  child.stdin.end()
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), delay(2000)])
  if (child.exitCode === null) child.kill('SIGTERM')
  lines.close()
  await rm(root, { recursive: true, force: true })
}
