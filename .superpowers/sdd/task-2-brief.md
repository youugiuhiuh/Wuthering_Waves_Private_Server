### Task 2: Add HTML info page + subscription headers + UA enrichment

**Files:**
- Create: `tools/sub-server/handler/page.go`
- Modify: `tools/sub-server/handler/subscription.go`

HTML info page: Embedded template showing proxy list with protocol/transport/tag, basic HTML/CSS.

Subscription headers in `writeResponse`:
- `Subscription-Userinfo`: upload/download/total/expiry bytes
- `Profile-Update-Interval`: seconds
- `Profile-Title`: base64-encoded title
- `Support-Url`, `Profile-Web-Page-Url`
