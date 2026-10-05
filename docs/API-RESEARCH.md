# Anonymous comment API research

Checked 2026-10-03 UTC against public, anonymous endpoints. This is observed behavior, not a vendor API contract. No authentication, cookies, account credentials, or access-control bypass was used. Request volume was deliberately small; this investigation did not re-crawl the sample level's complete main-comment history.

## Sources

- Official level frontend: <https://act.miyoushe.com/ys/ugc_community/mx/>
- Official comments frontend: <https://act.miyoushe.com/ys/ugc_community/level-detail/index.html#/comment?level_id=7257762194&region=cn_gf01>
- Official API wrappers (module 62433): <https://act.miyoushe.com/ys/ugc_community/level-detail/static/js/async/448.a6edb44c.js>
- Official main/child comment query implementation: <https://act.miyoushe.com/ys/ugc_community/level-detail/static/js/async/813.ce3bdf96.js>
- Metadata-only probe evidence: [api-probe-evidence.json](api-probe-evidence.json)

The source files were retrieved as static public files using HTTP, not by driving a browser. Asset hashes are point-in-time evidence and may change after site deployments.

## Main replies

### 2026-10-05 single-page HOT fallback

The user-reported level `75942301324` returns six main replies (`total: 6`) to a HOT request. Its first response is terminal (`has_more: false`, `next: "2"`), but reports `SORT_TYPE_FLOOR_DESC`. The earlier strict sort check rejected this complete page. The packaged backend now accepts only this main-list first-and-terminal-page case when the returned row count equals the declared total. It does not replay the differently sorted cursor. Missing totals, count shortfalls, continuation pages, nonterminal sort changes and child-thread sort changes remain invalid. A live packaged-backend collection made three anonymous requests and saved six main replies plus one child as a complete archive. Metadata evidence is in [api-probe-20261005.json](api-probe-20261005.json); no comment text or user details are retained.

`POST https://bbs-api.miyoushe.com/community/ugc_community/web/api/reply/list?lang=zh-cn`

```json
{
  "uid": "",
  "region": "cn_gf01",
  "level_id": "7257762194",
  "cursor": { "next": "", "size": 20, "sort_type": "SORT_TYPE_HOT" }
}
```

Existing plugin headers worked: JSON content type; origin `https://act.miyoushe.com`; referer `https://act.miyoushe.com/`; `x-rpc-client_type: 5`; `x-rpc-language: zh-cn`; the mobile UA already in `src/bbs.rs`. This investigation did not independently prove each header is required.

Successful envelope: `retcode: 0`, `message: "OK"`. Data keys observed: `can_reply`, `user_play_time`, `my_reply`, `reply_list`, `total`, `cursor`.

### Verified sort behavior

- `SORT_TYPE_HOT`: supported and used by the official main-list UI. The first 20-result page returned cursor `{"next":"20","has_more":true,"sort_type":"SORT_TYPE_HOT"}`. Replaying next `20` with size 5 returned five different main rows and next `25`. This sample behaves like an offset; keep the value opaque rather than calculating it.
- `SORT_TYPE_FLOOR_DESC`: supported for main replies. An initial size-5 page returned floors `1458,1457,1456,1455,1454`, with decreasing `created_at`, then cursor next `1453`. Replaying that cursor plus size 5 returned floors `1453,1452,1451,1450,1449`, then next `1448`. This is suitable for a recent-parent polling window, subject to omissions and live mutations.
- Guessed aliases `SORT_TYPE_TIME` and `SORT_TYPE_NEW` each returned HTTP 200 with application retcode `-502` and a generic retry-later message. They are **not verified usable**, and must not be used as if they were supported newest sorts. The generic error does not document the server's complete enum vocabulary.
- The current official main UI bundle only uses HOT; its child UI uses FLOOR_DESC. No distinct timestamp-sort option was found in those query implementations.

Responses omit `cursor.size`. The official client reconstructs each request cursor from response `next`, response `sort_type`, and its configured size 15. Preserve configured size explicitly. Do not combine a cursor from one sort with another sort. Do not interpret a floor ID as a global comment count or a child floor as the parent's floor.

Both tested main sorts reported `total: 1454` while the greatest main floor was 1458. Counts and floor numbers are therefore not interchangeable. Deleted, hidden, moderated, or otherwise excluded records can leave holes; their specific causes were not established here.

The older code comment about FLOOR_DESC missing 35 records is historical repository evidence, not reproduced by this low-volume probe. Keep HOT for full main-list reconciliation until completeness has been revalidated. A bounded newest-parent scan cannot prove absence of old main replies or discover new child replies on every old parent.

## Nested replies are truncated in the main list

`reply_list[*].sub_replies` is a preview, not a complete thread. On the sample HOT first page, main floor 5 reported `reply_stat.reply_count: "41"` but embedded only 2 child replies; another parent reported 13 and embedded only 2. Some parents reported 1 but embedded none. The missing preview rows are not evidence of deletion.

Consequences:

1. A complete main-page traversal is not a complete child-comment snapshot.
2. Never mark a previously archived child missing merely because it is absent from a main-list preview.
3. Child scans need their own successful traversal / coverage state and must be associated with the known parent ID from the request. Child samples have `f_reply_id: "0"`, so that field does not recover the parent.
4. A newest-parent polling window only refreshes children of the selected parents. Periodic complete main and child reconciliation is still needed for older-thread activity.

## Dedicated child endpoint

`POST https://bbs-api.miyoushe.com/community/ugc_community/web/api/level/reply/sub_replies?lang=zh-cn`

```json
{
  "uid": "",
  "region": "cn_gf01",
  "level_id": "7257762194",
  "parent_reply_id": "<main reply_id>",
  "cursor": { "next": "", "size": 15, "sort_type": "SORT_TYPE_FLOOR_ASC" }
}
```

The wrapper path and `parent_reply_id` field are from official frontend code. Anonymous probes confirm the endpoint returns the same reply-row shape under `data.reply_list`, plus `data.cursor`; no `total` was present in this response. Rows must be flattened as children of the requested parent, not as independent main comments.

### Important DESC cursor defect

The official child UI starts with `SORT_TYPE_FLOOR_DESC`, but the tested backend response does not provide a consistent DESC continuation:

- Request: empty next, size 5, FLOOR_DESC
- Returned child floors: `41,40,39,38,37`
- Returned cursor: `{"next":"38","has_more":true,"sort_type":"SORT_TYPE_FLOOR_ASC"}`
- Replaying this cursor with size 15 returned `38,39,40,41`, then `has_more:false`

Thus mechanically following the current official child's DESC-to-ASC sequence produced only five unique rows while the parent reported 41. Do not treat this endpoint traversal as complete merely because `has_more` became false. Do not synthesize a new cursor or override sort mid-traversal to hide the inconsistency.

### Verified complete ASC traversal on one thread

Starting a new child traversal with `SORT_TYPE_FLOOR_ASC`, empty next, and size 15 produced:

| Page | Child floors | Next | Has more |
| --- | --- | --- | --- |
| 1 | 1–15 | `16` | true |
| 2 | 16–30 | `31` | true |
| 3 | 31–41 | `42` | false |

All three response cursors retained FLOOR_ASC. There were exactly 41 distinct reply IDs, matching the parent’s reported count; no deeper nested children were returned. ASC therefore provides a verified complete traversal for this sampled thread and is the recommended starting sort for full child scans. This is one thread’s observed behavior, not a guarantee that all other threads have no moderation gaps, count lag, or malformed cursors. Continue to enforce incomplete-scan safeguards and compare observed child coverage with the parent’s count.

The plugin can use HOT for full main reconciliation, FLOOR_DESC for recent main polling, and FLOOR_ASC for dedicated child pagination. Main and child completeness should be tracked separately.

## Monitoring safety

- Deduplicate by `reply_id`; use content/field hashes for change detection, and retain parent relationships explicitly.
- Detect repeated cursors, repeated pages, unexpected sort changes, invalid envelopes, page limits, rate limits, and interrupted traversals. These are incomplete scans, never evidence of removal.
- Keep missing/unavailable observations separate from definitive deletion claims. The endpoint does not reveal why an old reply is absent.
- A live paginated API is not a transactional snapshot. HOT ranks, new replies, moderation, and response previews can change during a scan. Consider repeated complete coverage before reporting persistent absence.
- Returned `created_at` establishes creation time, not edit time. No edit timestamp or guaranteed change feed was verified.
