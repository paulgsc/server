
# Track Grouping Prompt

You are grouping learning resources into scheduler tracks for tabsched.

## What a track is

A track is the atomic scheduling unit. The scheduler maintains a
rolling window and ensures each track receives sessions proportional
to its target over that window. Resources within a track are visited
in a fixed cycle — session 1 → resource A, session 2 → resource B,
session 3 → resource A again, etc.

Tracks form a tree:
- **Internal tracks** (no resources): routing nodes that group
  related leaf tracks. They do not directly receive sessions.
- **Leaf tracks** (have resources): execution units. The scheduler
  picks a leaf and opens the next resource in its cycle.

A track is LEAF if and only if resource_labels is non-empty.
A track is INTERNAL if and only if resource_labels is empty.
A track CANNOT be both.

---

## Topology Construction (REQUIRED)

You MUST construct a valid tree. Follow this procedure in order:

1. Create leaf tracks that cover all resources.
   - Every input resource must appear in exactly one leaf track.
   - Each leaf track must have at least one resource.

2. Count the resulting leaf tracks:
   - If there is exactly one leaf track → it is also the root.
     Set its "parent" to null. Done.
   - If there are two or more leaf tracks → you MUST create exactly
     one internal root track (see step 3).

3. Create the root internal track:
   - "parent": null
   - "resource_labels": []
   - Assign every top-level leaf (or intermediate internal) track
     to this root by setting their "parent" to the root label.

4. Optionally create intermediate internal tracks to reflect domain
   grouping. Each intermediate internal track must itself have a
   parent (either the root or another internal track).

5. Before emitting JSON, verify:
   - Exactly one track has "parent": null.
   - Every other track references a label that appears in the output.
   - No track has both non-empty resource_labels AND child tracks.
   - Every input resource appears in exactly one leaf track.

Outputs that violate these rules are invalid. Do not emit them.

---

## Grouping principles

Apply these when deciding which resources share a leaf track:

1. **Orthogonal resources must be in separate tracks.**
   Resources with zero shared edges are cognitively unrelated. Each
   disconnected cluster of resources should be its own leaf track.

2. **Tightly connected resources belong in the same track.**
   Resources with edge weight ≥ 0.8 and kind "similar" are nearly
   interchangeable. Group these together.

3. **Reinforcing pairs can share a track or be adjacent sibling tracks.**
   If resource A reinforces resource B (weight ≥ 0.7), they can
   share a leaf track OR be in sibling leaf tracks under the same
   internal parent. Choose shared track when both are similar in
   scope; sibling tracks when one is significantly broader.

4. **Leaf track size: 1–5 resources.**
   Larger cycles dilute exposure. If a natural group exceeds 5,
   split it into sibling leaf tracks under a shared internal parent.

5. **Target proportional to resource count and domain priority.**
   Baseline: target = resource_count × 2 per leaf track.
   Adjust up for high-priority domains, down for lower priority.
   Internal track target = sum of children targets.
   Root track target = sum of all leaf targets.

6. **Preserve domain structure where reasonable.**
   Use the edge graph as the primary signal. The previous tracks
   summary is a continuity hint only — do not copy it.

---

## Input

### Resources and edge graph

{{RESOURCES_AND_EDGES}}

### Window size

{{WINDOW_SIZE}}

### Previous tracks (reference only — do NOT copy)

{{CURRENT_TRACKS}}

---

## Output requirements

Every track object MUST include ALL of these fields:

| Field            | Internal track          | Leaf track                |
|------------------|-------------------------|---------------------------|
| label            | unique string           | unique string             |
| parent           | null or parent label    | null or parent label      |
| target           | integer > 0             | integer > 0               |
| resource_labels  | [] (empty array)        | [non-empty array]         |
| derived_by       | "llm"                   | "llm"                     |
| rationale        | one sentence            | one sentence              |

The "parent" field MUST always be present. Use null explicitly for
the root. Never omit this field.

Emit tracks in BFS order: root first, then its children, then their
children, and so on.

---
## Output format

Respond with JSON only. No explanation outside the JSON object.
Do not use placeholder labels like "root", "leaf-example", or
"resource-a". All labels must derive from the actual input resources.

Schema:

{
  "tracks": [
    {
      "label":           <string — derived from actual input, unique>,
      "parent":          <string — parent track label> | null,
      "target":          <integer greater than 0>,
      "resource_labels": <array of actual input resource labels, or []>,
      "derived_by":      "llm",
      "rationale":       <string — one sentence>
    }
  ],
  "changes_from_current": [
    {
      "kind":        "moved | split | merged | retargeted | added | removed",
      "description": <string>
    }
  ]
}

