
# Track Grouping Prompt

You are grouping learning resources into scheduler tracks for tabsched.

## What a track is

A track is the atomic scheduling unit. The scheduler maintains a
rolling window and ensures each track receives sessions proportional
to its target over that window. Resources within a track are visited
in a fixed cycle — session 1 → resource A, session 2 → resource B,
session 3 → resource A again, etc.

A leaf track represents a **repeatable workflow loop**, not a topic
cluster. Ask: "would I naturally alternate between these resources
session-by-session?" If no, they should not share a leaf track.

Examples of valid workflow loops:
- "DFS problem drill" — problem list + active contest
- "Rust memory internals" — docs page + OSS source + chat notes
- "Korean study loop" — textbook + grammar reference + vocab drill
- "Korean immersion" — drama episode + media playlist

Tracks form a tree:
- **Internal tracks** (no resources): routing nodes that group
  related leaf tracks. They do not directly receive sessions.
- **Leaf tracks** (have resources): execution units. The scheduler
  picks a leaf and opens the next resource in its cycle.

A track is LEAF if and only if resource_labels is non-empty.
A track is INTERNAL if and only if resource_labels is empty.
A track CANNOT be both.

---

## Fixed Segment Layer (REQUIRED — enforce before all else)

You MUST organize all tracks under the following fixed top-level segments.
These are the ONLY permitted direct children of the root:

| Segment label      | What belongs here                                        |
|--------------------|----------------------------------------------------------|
| `math`             | Pure math, proof-based texts, analysis, discrete math    |
| `dsa`              | Algorithms, data structures, LeetCode, competitive prog  |
| `systems`          | Rust, kernel, networking, memory, compilers, low-level   |
| `infra`            | NixOS, DevOps, monitoring, local tooling, config         |
| `language`         | Korean, HSK, any natural language — study and media      |
| `career`           | Job applications, resume prep, interview resources       |
| `leisure`          | Drama, music, passive media with no study intent         |

Rules — these are not suggestions:

1. These seven segments MUST appear as direct children of root.
2. Every resource MUST be assigned to exactly one segment.
3. You MUST NOT create additional top-level categories beyond these seven.
4. You MUST NOT rename, merge, or omit any segment, even if it has zero
   resources. A segment with zero resources still appears as an internal
   node with no children.
5. All leaf tracks must descend from one of these segments (possibly
   via intermediate internal nodes).
6. If a resource does not clearly fit any segment, assign it to the
   closest one by primary intent. When in doubt, use the resource URL
   and title — not the embedding similarity — to decide.

---

## Topology Construction (REQUIRED)

You MUST construct a valid tree. Follow this procedure in order:

1. For each segment, collect all resources that belong to it.

2. Within each segment, form leaf tracks:
   - Every resource in the segment must appear in exactly one leaf track.
   - Each leaf track must have 1–5 resources.
   - Resources in a leaf track must form a coherent workflow loop
     (see definition above). Do not group by topic alone.
   - If a segment has zero resources, emit it as a childless internal
     node with target = 0.

3. If a segment has more than one leaf track, you MAY create intermediate
   internal nodes within the segment to sub-group them. This is optional.

4. Create the root internal track:
   - "parent": null
   - "resource_labels": []
   - The seven segments are its direct children.

5. Before emitting JSON, verify:
   - Exactly one track has "parent": null (the root).
   - The root has exactly seven children, one per segment.
   - Every other track references a label that appears in the output.
   - No track has both non-empty resource_labels AND child tracks.
   - Every input resource appears in exactly one leaf track.
   - No resource appears in more than one leaf track.

Outputs that violate these rules are invalid. Do not emit them.

---

## Grouping principles

Apply these when deciding how to form leaf tracks within a segment:

1. **Orthogonal resources within a segment go in separate tracks.**
   Even within the same segment, unrelated workflow loops should not
   be merged. A Rust docs page and a Rust OSS codebase are both
   `systems`, but if they belong to different work loops, split them.

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

5. **Cross-domain grouping is forbidden.**
   Resources assigned to different segments MUST NOT share a leaf
   track, regardless of edge weight or embedding similarity. A math
   PDF and a YouTube explainer that happen to be similar are still
   in different segments.

6. **Avoid generic or catch-all labels.**
   Track labels like "general", "misc", "multimedia", "mixed", or
   "learning" are invalid. Every label must reflect a specific
   workflow loop within a known segment. Good: "rust-memory-loop",
   "dfs-contest-drill", "korean-textbook-cycle". Bad: "general-study".

7. **Targets are calculated per segment priority.**
   Use these domain multipliers:

   | Segment   | Multiplier |
   |-----------|------------|
   | math      | 1.2        |
   | dsa       | 1.2        |
   | systems   | 1.3        |
   | infra     | 1.0        |
   | language  | 1.0        |
   | career    | 0.8        |
   | leisure   | 0.6        |

   Leaf target = ceil(resource_count × 2 × multiplier).
   Internal/segment target = sum of children targets.
   Root target = sum of all segment targets.

8. **Preserve continuity where reasonable.**
   The previous tracks summary is a continuity hint only. Do not
   copy it. Use the edge graph as primary signal.

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

Emit tracks in BFS order: root first, then its seven segment children,
then their children, and so on.

---

## Output format

Respond with JSON only. No explanation outside the JSON object.
Do not use placeholder labels like "root", "leaf-example", or
"resource-a". All labels must derive from the actual input resources.

Schema:

```json
{
  "tracks": [
    {
      "label":           "<string — derived from actual input, unique>",
      "parent":          "<string — parent track label> | null",
      "target":          "<integer greater than 0>",
      "resource_labels": "<array of actual input resource labels, or []>",
      "derived_by":      "llm",
      "rationale":       "<string — one sentence>"
    }
  ],
  "changes_from_current": [
    {
      "kind":        "moved | split | merged | retargeted | added | removed",
      "description": "<string>"
    }
  ]
}
```
