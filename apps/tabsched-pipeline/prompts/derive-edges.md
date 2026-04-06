# Edge Derivation Prompt

You are analyzing a set of learning resources to derive relationships
between them for a spaced-repetition scheduler called tabsched.

## What tabsched does

tabsched rotates between "tracks" (groups of resources) to ensure
balanced cognitive exposure across learning domains. Resources in the
same track are visited in a fixed cycle. Resources in different tracks
compete for scheduler slots.

The edge graph you produce determines how the scheduler groups
resources into tracks. Your job is to identify which resources share
enough conceptual overlap that working on one is likely to reinforce
the other.

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
     different domains. Example: a Rust OSS repo and a Rust docs page
     both use lifetimes but serve different purposes.

2. **Reject** — the pair appeared similar by embedding but the
   relationship is not meaningful for learning. For example:
   - Two job application tabs (similar surface, no knowledge transfer)
   - A language learning tab and a math tab that both happen to use
     similar vocabulary

3. **Add** — you notice a relationship not in the candidate list that
   you believe is meaningful. Only add if confidence is high.

## Weight

Weight (0.0–1.0) represents how strongly the relationship holds:

- 0.9–1.0: nearly identical subject matter or extremely strong reinforcement
- 0.7–0.8: clear relationship, frequent reinforcement
- 0.5–0.6: weak but real connection
- Below 0.5: not worth adding

## Output format

Respond with JSON only. No explanation text outside the JSON object.

```json
{
  "edges": [
    {
      "source": "resource-label-a",
      "target": "resource-label-b",
      "kind": "similar" | "reinforces" | "overlaps",
      "weight": 0.0–1.0,
      "reason": "one sentence explanation"
    }
  ],
  "rejected": [
    {
      "source": "resource-label-a",
      "target": "resource-label-b",
      "reason": "why this pair is not meaningful"
    }
  ]
}
```

Edges are undirected. List each pair once (source < target alphabetically).
Do not include the `rejected` field if it is empty.

