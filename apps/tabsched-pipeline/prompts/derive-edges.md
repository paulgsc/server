
# Edge Derivation Prompt

You are analyzing a set of learning resources to derive relationships
between them for a spaced-repetition scheduler called tabsched.

## What tabsched does

tabsched rotates between "tracks" (groups of resources) to ensure
balanced cognitive exposure across a fixed set of life domains.
Resources in the same track are visited in a fixed cycle. Resources
in different tracks compete for scheduler slots.

The edge graph you produce determines how the scheduler groups
resources into tracks within a domain. Your job is to identify which
resources share enough conceptual overlap that working on one is
likely to reinforce the other.

**Important constraint:** tabsched organizes all resources under
exactly seven fixed top-level domains:

| Domain    | What belongs here                                        |
|-----------|----------------------------------------------------------|
| math      | Pure math, proof-based texts, analysis, discrete math    |
| dsa       | Algorithms, data structures, LeetCode, competitive prog  |
| systems   | Rust, kernel, networking, memory, compilers, low-level   |
| infra     | NixOS, DevOps, monitoring, local tooling, config         |
| language  | Korean, HSK, any natural language — study and media      |
| career    | Job applications, resume prep, interview resources       |
| leisure   | Drama, music, passive media with no study intent         |

Edges between resources in **different domains are meaningless** for
the scheduler. Even if two resources are embedding-similar, a cross-
domain edge will not affect grouping — the domain boundary is a hard
wall. Do not add or accept cross-domain edges.

## Resources

{{RESOURCES}}

## Candidate relationships (from embedding similarity)

These pairs scored above the similarity threshold. Similarity is
cosine similarity of embedded content (title + headings + keywords).

{{CANDIDATE_EDGES}}

## Your task

For each candidate pair, decide:

1. **Accept with kind** — the relationship is meaningful for learning
   - `similar`: same subject matter, roughly interchangeable. Doing
     either in a session covers similar ground.
   - `reinforces`: working on source makes target easier to absorb.
     The relationship is directional but not strictly sequential.
   - `overlaps`: they share a concept or technique but are otherwise
     different in scope or purpose within the same domain.

2. **Reject** — the pair is not meaningful for scheduling. Reject if:
   - The resources belong to different domains (hard rule — always reject)
   - The similarity is superficial (similar vocabulary, not similar knowledge)
   - Working on one would not affect readiness for the other
   - Examples: two job tabs (no knowledge transfer), a drama tab and
     a grammar tab that share character names, a math PDF and a
     YouTube video that use similar notation but target different goals

3. **Add** — you notice a same-domain relationship not in the
   candidate list that you believe is meaningful. Only add if
   confidence is high (weight ≥ 0.7) and both resources are in
   the same domain.

## Weight

Weight (0.0–1.0) represents how strongly the relationship holds:

- 0.9–1.0: nearly identical subject matter or extremely strong reinforcement
- 0.7–0.8: clear relationship, frequent reinforcement
- 0.5–0.6: weak but real connection
- Below 0.5: not worth adding — reject instead

## Domain assignment (emit alongside edges)

For each resource, emit its domain assignment. This is used
downstream by the track grouper to enforce the segment layer.

Use the resource's URL, title, and keywords — not embedding similarity
— to determine domain. When the URL and title conflict, prefer the
title. When ambiguous, assign to the domain that reflects the
**primary cognitive intent** of opening that tab.

## Output format

Respond with JSON only. No explanation text outside the JSON object.

```json
{
  "domain_assignments": [
    {
      "resource": "<resource-label>",
      "domain": "math | dsa | systems | infra | language | career | leisure",
      "reason": "<one sentence — primary intent of this resource>"
    }
  ],
  "edges": [
    {
      "source": "<resource-label-a>",
      "target": "<resource-label-b>",
      "kind": "similar | reinforces | overlaps",
      "weight": 0.0,
      "reason": "<one sentence explanation>"
    }
  ],
  "rejected": [
    {
      "source": "<resource-label-a>",
      "target": "<resource-label-b>",
      "reason": "<why this pair is not meaningful — note if cross-domain>"
    }
  ]
}
```

Edges are undirected. List each pair once (source < target alphabetically).
Do not include the `rejected` field if it is empty.
Cross-domain edges must always appear in `rejected`, never in `edges`.
