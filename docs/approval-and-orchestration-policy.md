# Approval And Orchestration Policy

## Status Quo

Spine currently receives `requires_approval`, but the live HTTP/WebSocket runtime does not enforce it. Tasks plan and dispatch immediately. The planner also emits a single step per intent, with no decomposition or reviewer lane yet.

## 1. Approval Policy

### Threshold Matrix

| Risk | CLI (operator) | API/Web | Discord |
| --- | --- | --- | --- |
| low | auto-approve | auto-approve | auto-approve |
| medium | auto-approve | auto-approve | hold |
| high | auto-approve | hold | hold |
| critical | hold | hold | hold |

CLI means Devon has physical access. That is the highest-trust surface.

Environment override:
- `HEIWA_AUTO_APPROVE=all`: auto-approve everything for development
- `HEIWA_AUTO_APPROVE=cli`: default, keeps the CLI high-risk override

### Enforcement Point

The approval gate lives in Spine after planning and before dispatch.

Pseudo-flow:

```python
if requires_approval and not auto_approved(surface, risk):
    registry.add(task_id, payload)
    speak(TASK_STATUS, status="AWAITING_APPROVAL")
    return

async def handle_approval_decision(data):
    state = registry.decide(task_id, approved, actor, reason)
    if state.status == "APPROVED":
        payload = registry.consume_payload(task_id)
        dispatch(payload)
    elif state.status == "REJECTED":
        speak(TASK_STATUS, status="REJECTED")
```

### Approval Surfaces

All surfaces should converge on the same approval registry and decision path:
- CLI: `heiwa approve <task_id>` / `heiwa reject <task_id>`
- HTTP: `POST /tasks/{id}/approve` / `POST /tasks/{id}/reject`
- Discord: approval buttons in Messenger
- App right rail: approve/reject buttons

## 2. Decomposition Rules

### Stay Single-Lane When

- `chat`, `general`, or `status_check`
- low risk and high confidence
- short prompt under 15 words, unless explicitly build/deploy/strategy
- bounded audit work that already maps to a dedicated script path

### Decompose When

- deploy intent: preflight -> deploy -> health check -> report
- build with multiple artifacts: implement + test
- explicit multi-part requests: “and then”, “after that”, numbered sequences
- high-risk complex work: high risk plus long input

Cap decomposition at 4 steps.

### Step Execution Model

Sequential by default, with an active task state machine:

```python
@dataclass
class ActiveTask:
    task_id: str
    steps: list[dict]
    current_step_index: int = 0
    results: list[dict] = field(default_factory=list)
```

`PASS` advances. `FAIL` halts.

## 3. Multi-Lane Policy

Only two cases should introduce parallel or multi-lane work:

1. Independent sub-tasks
   Steps with no data dependency may share a `parallel_group` and run together.

2. Reviewer lane for high-risk execution
   High-risk `build` or `deploy` work should append a cheaper review step after execution.

What not to build:
- no autonomous agent spawning
- no speculative parallel provider fan-out
- no hidden inter-task coordination

## 4. App Right Rail UX

Top to bottom:
- Plan: current steps, route, risk, status
- Approvals: pending queue with approve/reject actions
- Artifacts: produced files and outputs
- Activity: latest task status events

The app should consume one authenticated operator stream rather than invent a separate orchestration path.

## 5. Implementation Order

### Phase A: Approval Gating

1. Add the approval gate in Spine
2. Add approval decision handling
3. Add `auto_approved(surface, risk)`
4. Wire `HEIWA_AUTO_APPROVE`
5. Add CLI and HTTP approve/reject controls

### Phase B: Multi-Step Decomposition

1. Detect compound requests
2. Add active task state to Spine
3. Advance or halt on step results

### Phase C: Reviewer Lane

1. Add `parallel_group` to step plans
2. Append review for high-risk build/deploy work
3. Use a cheaper review tier than the primary execution tier

### Phase D: App Right Rail

1. Add an authenticated operator websocket
2. Build plan, approvals, artifacts, and activity panels
3. Wire app approve/reject actions to hub endpoints

Phase A is the highest priority because `requires_approval` is otherwise theater.
