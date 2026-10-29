# Deterministic recovery runs

Recovery runs are included from **`v0.1.1-alpha.3`**. Publication and verification
are recorded in the [prerelease notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3).
Earlier releases do not include this API.

A recovery run gives one synthetic client test its own finite response sequence,
such as HTTP `503`, `503`, then `200`. Each admitted attempt receives a sequence
position before waiting. A reset starts that sequence again in a new generation.
Existing `/sleep` and `/simulate` routes keep their stateless behavior.

From **`v0.1.1-alpha.4`**, steps support optional `fixture` and `fault` fields. See
[response fixtures and transport faults](response-fixtures.md) for their
contract and limits; alpha.3 supports only the original JSON steps.

Runs are volatile, process-local state. Restarting SleepAPI loses every run and
receipt. Run identifiers are server-generated identifiers, not secrets or
credentials. SleepAPI has no authentication: keep it on a trusted local interface.
A run must be used with the same server process that created it.

## Create a run

`GET /runs` reports `active_runs` and the fixed run limits. It does not enumerate
run identifiers.

```sh
curl -sS http://127.0.0.1:3000/runs \
  -H 'Content-Type: application/json' \
  -d '{"sequence":[{"status_code":503,"duration_ms":50},{"status_code":503,"duration_ms":50},{"status_code":200,"duration_ms":50}],"ttl_ms":300000}'
```

`POST /runs` returns `201` with the new run's identifier and generation. Each
sequence step requires `status_code`; `duration_ms` defaults to `400` and
`method` defaults to `tokio_sleep`. The other methods are `tokio_spawn`,
`thread_sleep`, and `thread_park`. Status and duration validation follows the
existing simulation rules. The entire sequence is validated before creating a
run; unknown fields are rejected.

Creation, inspection, and reset return the same summary fields:

| Field | Meaning |
| --- | --- |
| `run_id` | Server-generated identifier, such as `run-1` |
| `generation` | Current generation, starting at 1 |
| `attempts_started` | Lifetime admitted attempts across all generations |
| `generation_attempts` | Admitted attempts in the current generation |
| `ttl_ms` | Lifetime selected at creation |
| `expires_in_ms` | Remaining lifetime at the snapshot, rounded down to milliseconds |
| `sequence` | Fully resolved steps, including default delay and method |
| `retained_receipts` | Number of currently retained receipts |

Save the returned `run_id` and `generation`. The examples below use `run-1` and
`1`; replace them with the values returned by your server.

```sh
RUN_ID=run-1
GENERATION=1
```

`ttl_ms` is a fixed lifetime measured from creation, not an idle timeout. Activity
and reset do not extend it.

## Execute the sequence

Every attempt supplies the generation it expects. For the run above, these three
requests return `503`, `503`, and `200`, respectively:

```sh
for attempt in 1 2 3; do
  curl -i -sS "http://127.0.0.1:3000/runs/${RUN_ID}/attempts" \
    -H 'Content-Type: application/json' \
    -d "{\"generation\":${GENERATION}}"
done
```

`POST /runs/{run_id}/attempts` accepts only the expected generation; the chosen
step defines the response status, delay, and waiting method. Its response uses
the simulation JSON contract with run, generation, attempt, and sequence
correlation metadata. `attempt` and `sequence_step` are both one-based within the
current generation; use `(run_id, generation, attempt)` to identify an attempt
across resets. Provider failures are simulated responses, not errors in run
management.

The sequence is finite. A fourth attempt against the three-step example receives
a structured `409` API error; it does not silently repeat the final success.
Create a new run or reset the existing one to exercise the sequence again.

## Inspect and reset

```sh
curl -sS "http://127.0.0.1:3000/runs/${RUN_ID}"
curl -sS "http://127.0.0.1:3000/runs/${RUN_ID}/receipts"

curl -sS "http://127.0.0.1:3000/runs/${RUN_ID}/reset" \
  -H 'Content-Type: application/json' \
  -d "{\"generation\":${GENERATION}}"
```

Use the generation returned by reset for subsequent attempts and resets. A
request with a stale generation receives `409` and consumes no attempt. This
prevents a delayed retry from silently entering the next test generation.

Reset advances the generation, clears retained receipts, and rewinds the
sequence. It does not replenish the run's lifetime attempt budget, extend its
expiry, or cancel already-admitted work. Such work keeps its original generation
and selected response; late completion or cancellation cannot populate the new
generation's receipts.

Delete a run when the test finishes:

```sh
curl -i -sS -X DELETE "http://127.0.0.1:3000/runs/${RUN_ID}"
```

Deletion returns `204` with no body and removes the run and its retained
receipts. It requires no generation body and deletes the currently stored run,
even if another caller has reset it. It does not cancel admitted work. A late result cannot recreate the deleted run. Expired runs have the same
lookup behavior as absent runs: `404`. Expiry is enforced using a monotonic
clock on run operations and attempt cleanup; expired entries are pruned before
new runs compete for capacity. There is no independent background sweeper.

## Concurrent attempts and cancellation

An attempt consumes a position only after its request is valid, its expected
generation matches, and it has obtained a shared simulation-capacity slot. The
server assigns the position atomically under the run-state lock. This ordering
is the reservation order, not network arrival order or response completion order.
No run-state lock is held while waiting.

Concurrent attempts get distinct positions. A later, shorter step can finish
before an earlier, longer step without changing either assignment. Different
runs have separate sequence state but share the same simulation capacity with
the stateless routes. Creating, inspecting, resetting, and deleting runs do not
consume simulation slots.

Invalid requests, stale generations, exhausted sequences, and rejected capacity
admission consume no position. Once admitted, an attempt is consumed even if its
handler is cancelled; cancellation does not roll the sequence back. Async waits
release their capacity when cancelled. An already-started blocking wait can
continue and retain its capacity slot after its handler disappears, as on the
stateless routes. The process shutdown deadline still applies.

## Bounded receipts

`GET /runs/{run_id}/receipts` returns `run_id`, `generation`, and a `receipts`
array. Each receipt contains `generation`, `attempt`, `sequence_step`,
`selected_status_code`, `method`, `requested_duration_ms`, `outcome`, and
`elapsed_ms`. Run ID comes from the enclosing object.

Outcomes are `pending`, `completed`, `handler_cancelled`, or `wait_failed`.
`elapsed_ms` is `null` while pending; afterwards it records server-side time
since reservation, including scheduling. The selected status is the configured
simulation response; it does not become a different status when a wait fails.
A selected `503` can have outcome `completed`: the simulated failure was produced
successfully. A waiting-task failure instead produces a structured HTTP `500` API
error and a `wait_failed` receipt, if that receipt is still retained.

Receipts describe the assigned step and the server-side attempt outcome. They
are diagnostics, not a durable event log. A completed attempt means the server
finished its simulation and prepared a response; it does not prove that the
client received the response. A handler-cancellation outcome does not prove that
already-started blocking work stopped.

Only the most recent 32 admitted attempts per run are retained, in assignment
order. Older receipts can be evicted while their work is still running. Later
completion does not reinsert an evicted receipt. Reset clears receipts, and
expiry or deletion removes the run's diagnostic state. Retrieve the bounded
snapshot before cleanup if a test needs to retain it externally.

## Limits and errors

| Bound | Limit and behavior |
| --- | --- |
| Stored live runs | 64 per server process; expired runs are pruned before applying the limit; full capacity rejects creation with `429` |
| Sequence | 1–64 steps; empty or oversized sequences are rejected |
| Lifetime admitted attempts | 1,024 per run across all generations; reset does not replenish this budget; further attempts return `409` |
| Retained receipts | Most recent 32 admitted attempts per run; older records are evicted |
| Lifetime | `ttl_ms` defaults to 300,000; accepted range is 1–3,600,000 ms |
| Step delay | Existing `SLEEPAPI_MAX_DURATION_MS` limit; default 30,000 ms |
| Shared active waits | Existing `SLEEPAPI_MAX_IN_FLIGHT` limit; default 64 slots |
| Request body | Existing 8 KiB JSON-body limit, including run creation |

Run-count, step-count, attempt, and receipt bounds are fixed in this initial
implementation. Request-level TTL can select a shorter lifetime within the
stated range. Expiry and deletion can remove run state while admitted waits are
still active; these waits remain covered by shared capacity and shutdown limits.

Malformed JSON returns `400`, missing or unsupported JSON content type returns
`415`, invalid schema or values return `422`, and oversized request bodies return
`413`. Unknown or expired runs return `404`; stale generations, exhausted
sequences, and exhausted lifetime attempt budgets return `409`. A full run store
rejects creation with `429` and error code `rate_limited`. Shared
simulation-capacity rejection returns `429` with error code `capacity_exceeded`.
Both `429` responses carry `Retry-After: 1`.

For attempts, JSON validation happens first, then shared-capacity admission,
then run lookup and generation/budget checks. When every simulation slot is busy,
`429` therefore takes precedence over a missing-run `404` or a run-state `409`.
None of these rejections consumes a sequence position. API errors have
`simulated: false`, distinct from a selected simulated `503` or other failure.

Exact fixture bodies and transport faults are separate enhancements. Recovery
runs currently return complete JSON simulation responses.
