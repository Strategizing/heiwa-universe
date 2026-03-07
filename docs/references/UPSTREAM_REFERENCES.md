# Upstream References

> These replace the full repo clones formerly in `docs_and_deps/`.  
> Fetch docs on-demand instead of cloning ~900 MB of upstream repos.

## Core Dependencies

| Project | Repo | Version | Docs URL | Notes |
|:--------|:-----|:--------|:---------|:------|
| **LiteLLM** | [github.com/BerriAI/litellm](https://github.com/BerriAI/litellm) | latest | [docs.litellm.ai](https://docs.litellm.ai) | Proxy layer for multi-provider routing |
| **vLLM** | [github.com/vllm-project/vllm](https://github.com/vllm-project/vllm) | latest | [docs.vllm.ai](https://docs.vllm.ai) | High-throughput inference engine (Node B) |
| **SpacetimeDB** | [github.com/clockworklabs/SpacetimeDB](https://github.com/clockworklabs/SpacetimeDB) | 1.0.x | [spacetimedb.com/docs](https://spacetimedb.com/docs) | Phase D state layer |
| **SpacetimeDB Python SDK** | [github.com/clockworklabs/spacetimedb-python-sdk](https://github.com/clockworklabs/spacetimedb-python-sdk) | latest | See repo README | Python client bindings |
| **cloudflared** | [github.com/cloudflare/cloudflared](https://github.com/cloudflare/cloudflared) | latest | [developers.cloudflare.com/cloudflare-one/connections/connect-networks](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks) | Zero-trust tunnel |
| **pgvector** | [github.com/pgvector/pgvector](https://github.com/pgvector/pgvector) | latest | See repo README | PostgreSQL vector extension |

## Infrastructure

| Service | Dashboard | Docs |
|:--------|:----------|:-----|
| **Railway** | [railway.app](https://railway.app) | [docs.railway.app](https://docs.railway.app) |
| **Cloudflare** | [dash.cloudflare.com](https://dash.cloudflare.com) | [developers.cloudflare.com](https://developers.cloudflare.com) |
| **NATS** | — | [docs.nats.io](https://docs.nats.io) |
| **E2B** | [e2b.dev](https://e2b.dev) | [e2b.dev/docs](https://e2b.dev/docs) |
| **SiliconFlow** | [siliconflow.com](https://siliconflow.com) | [siliconflow.com/articles](https://siliconflow.com/articles) |

## AI Model References

| Model | Provider | Context | Notes |
|:------|:---------|:--------|:------|
| Llama 4 Scout (MoE) | Meta/Ollama | 128K | Primary M4 Pro reasoning model |
| GLM-4.7-Flash | THU/Ollama | 128K | Coding specialist, 24GB viable |
| Qwen2.5-Coder-7B | Alibaba/Ollama | 32K | Node B coding model |
| all-MiniLM-L6-v2 | SentenceTransformers | — | Embedding model for Node B |

## When to Clone

Only clone an upstream repo if you need to:
1. Debug a build failure that requires stepping through source
2. Contribute a patch upstream
3. Need offline access to docs for an extended period

Otherwise, use the docs URLs above.
