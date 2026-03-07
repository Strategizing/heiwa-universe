# Hardware Constraints — Heiwa Sovereign Mesh

> Source: Enterprise Architecture Analysis, 2026-03-06

## The Context Cliff (24 GB Unified Memory)

The M4 Pro's 24 GB unified memory is shared between:
1. **Model weights** — Q4 quantized at ~0.5 bytes/param
2. **KV cache** — grows with context length and conversation history
3. **Thinking tokens** — reasoning models allocate extra working memory
4. **macOS overhead** — ~4-5 GB minimum

### Memory Budget Formula

```
Available_for_model = 24GB - OS_overhead(~5GB) = ~19GB
Model_weights(Q4)   = params_B × 0.5 GB
KV_cache_budget     = Available - Model_weights
```

| Model | Params | Q4 Weights | KV Budget | Viable? |
|:------|:-------|:-----------|:----------|:--------|
| 7B    | 7B     | ~3.5 GB    | ~15.5 GB  | ✅ Comfortable |
| 14B   | 14B    | ~7 GB      | ~12 GB    | ⚠️ Tight with heavy context |
| 30B+  | 30B    | ~17+ GB    | ~2 GB     | ❌ Unviable — no context headroom |

### Why MoE Models Win

Mixture of Experts (e.g., Llama 4 Scout) have massive total param counts but only activate
a small subset (3-8B) per inference pass. This means:
- **Quality** = GPT-4 class (from total knowledge encoded in all experts)
- **Memory** = Small model footprint (only active expert weights in memory)
- **KV headroom** = Plenty of room for deep agentic context

### The Swap Death Spiral

When unified memory is exhausted, macOS swaps to SSD. Token generation collapses from
~50 tok/s to unusable speeds. This is not gradual degradation — it's a cliff.

**Hard rule:** If `memory_pressure` hits critical, immediately downgrade to smaller model
or route to cloud inference. Never let an agent run in swap.

## Power Efficiency

M4 Pro under heavy LLM inference: ~1/10th the wattage of a discrete GPU.
This enables continuous, untethered agentic operations without draining battery.

## Node B Constraints (RTX 3060, 12 GB VRAM)

- Strict 12 GB VRAM ceiling
- Use only for: embeddings, reranking, browser automation, media generation, small model inference
- Models must be ≤8B params at Q4 to leave VRAM for other GPU tasks
- Operates as headless worker — no interactive use
