# Track Grouping Prompt

You are grouping learning resources into scheduler tracks for tabsched.

## What a track is

A track is the atomic scheduling unit. The scheduler maintains a
rolling window and ensures each track receives sessions proportional
to its `target` over that window. Resources within a track are visited
in a fixed cycle — session 1 → resource A, session 2 → resource B,
session 3 → resource A again, etc.

Tracks form a two-level tree:

- **Internal tracks** (no resources): routing nodes that group
  related leaf tracks. They do not directly receive sessions.
- **Leaf tracks** (have resources): execution units. The scheduler
  picks a leaf and opens the next resource in its cycle.

## Grouping principles

Apply these in order of priority:

1. **Orthogonal resources must be in separate tracks.**
   Resources with zero shared edges are cognitively unrelated. If
   they share a track, one will starve the other within the cycle.
   Each orthogonal cluster of resources should be its own leaf track.

2. **Tightly connected resources belong in the same track.**
   Resources with edge weight ≥ 0.8 and kind `similar` are nearly
   interchangeable — visiting either covers the same ground. Group
   these together. This is the main way to keep track count manageable.

3. **Reinforcing pairs can share a track or be adjacent tracks.**
   If resource A `reinforces` resource B (weight ≥ 0.7), they can
   share a track (so they appear in alternating cycle slots) OR be
   in sibling leaf tracks under the same internal parent. Choose
   shared track when both are roughly equal in scope; separate tracks
   when one is significantly broader.

4. **Leaf track size: 1–5 resources.**
   Larger cycles dilute exposure per resource. If a natural group
   exceeds 5, split it into two sibling leaf tracks.

5. **Target proportional to resource count and domain priority.**
   Baseline: `target = resource_count * 2` per leaf track.
   Adjust up for high-priority domains, down for lower priority.
   Internal track target = sum of children targets.
   Root track target = sum of all domain targets.

6. **Preserve the user's domain structure where reasonable.**
   The user has defined 6 top-level domains. Respect these as
   internal track groupings unless the edge graph strongly suggests
   a cross-domain relationship that should be co-scheduled.

## Input

### Resources and edge graph

{{RESOURCES_AND_EDGES}}

### User's current tracks (soft constraints)

Treat these as the user's expressed intent. You may reorganize leaf
tracks and adjust targets, but explain any change to the top-level
domain structure.

{{CURRENT_TRACKS}}

### Window size

{{WINDOW_SIZE}}

## Output format

Respond with JSON only. No explanation text outside the JSON object.

```json
{
  "tracks": [
    {
      "label": "string — human readable, unique",
      "parent": "string — parent track label, or null for root",
      "target": "number — sessions per window, > 0",
      "resource_labels": ["array of resource labels, in cycle order"],
      "derived_by": "llm",
      "rationale": "one sentence — why this grouping"
    }
  ],
  "changes_from_current": [
    {
      "kind": "moved" | "split" | "merged" | "retargeted" | "added" | "removed",
      "description": "what changed and why"
    }
  ]
}
```

Rules:

- Emit exactly one root track (parent = null).
- Internal tracks have empty `resource_labels` array.
- Leaf tracks have non-empty `resource_labels`.
- A track cannot have both resource_labels and child tracks.
- All resource labels in the output must appear in the input.
- Every input resource must appear in exactly one leaf track.
- Emit tracks in BFS order (parents before children).

