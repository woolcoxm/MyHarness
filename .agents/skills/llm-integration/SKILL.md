---
name: llm-integration
description: Use whenever building features that call LLM APIs — chat completion, streaming, function calling, embeddings, RAG pipelines, prompt engineering for applications, token optimization, or handling model responses in production code. Also when debugging LLM integration issues like inconsistent outputs, token limit errors, or streaming failures.
---

# LLM Integration in Applications

Integrating LLMs into real software is not the same as chatting with them. Production LLM features face: unpredictable latency, nondeterministic outputs, token budgets, streaming failures, and the hallucination problem. This skill covers the patterns that make LLM features reliable.

## API Integration Patterns

### Client design
```
LLM Client
├── Retry with exponential backoff (429, 5xx, network)
│   └── Honor Retry-After header; cap at 5 attempts
├── Timeout (connect 10s, total stream 180s)
│   └── A stalled stream must fail loudly, not hang
├── Rate limiting (token bucket per API key)
│   └── Share a single limiter across all calls
└── Circuit breaker (open after 5 consecutive failures)
    └── Half-open after 30s; log every state transition
```

### Streaming vs request-response
| Pattern | When | Tradeoffs |
|---|---|---|
| Request-response | Backend batch jobs, classification | Simple, but blocks; full output before any result |
| SSE streaming | User-facing chat, progressive display | Fast first token; harder error handling; connection drops |
| WebSocket | Bidirectional (voice, real-time) | Complex; overkill for text-only |

**Streaming rule**: always handle partial responses. If the stream drops mid-generation, you have a partial result — decide whether to show it, retry, or error. Never show the user a spinner forever.

### Function/tool calling
```python
# Define tools the model can call
tools = [
    {
        "name": "get_weather",
        "description": "Get current weather for a city",
        "parameters": {
            "type": "object",
            "properties": {
                "city": {"type": "string", "description": "City name"},
            },
            "required": ["city"]
        }
    }
]

# The model returns a tool_call; you execute it and return the result
# Key: validate the model's arguments against your schema before executing
import json
if tool_call.name == "get_weather":
    args = validate_or_error(tool_call.arguments, schema)
    result = weather_api.get(args["city"])
    # Return the result for the next model turn
```

**Never trust model-generated arguments blindly.** The model can hallucinate parameter names, send wrong types, or omit required fields. Validate at the boundary.

### Token counting and budgets
- Input tokens = system prompt + messages + tool definitions (yes, tools cost tokens on EVERY request)
- Output tokens = the model's response (including reasoning/thinking tokens)
- Context window = the maximum total (input + output) the model can handle
- **Track both separately.** Input dominates cost for long conversations; output dominates for generation tasks.

**Cost formula**: `(input_tokens × input_price) + (output_tokens × output_price) + (cache_read_tokens × cache_price)`. Cache reads typically cost 10% of fresh input.

## Prompt Engineering for Applications

### System prompts (the contract)
The system prompt is your API contract with the model. Structure it:
```
1. Identity and role (what the model IS)
2. Capabilities and constraints (what it CAN and CANNOT do)
3. Output format specification (exact format, with an example)
4. Domain context (project-specific information)
5. Behavioral rules (safety, tone, edge cases)
```

**Rule**: the system prompt should be byte-stable for the session (for prompt caching) and specific enough that a reasonable person could follow it. If you write "be helpful and concise" without defining what "concise" means in tokens, you've written nothing.

### Structured output (the reliable pattern)
For programmatic consumption, require JSON:
```
Return ONLY a JSON object matching this schema:
{
  "answer": "string — the direct answer",
  "confidence": "number 0-1",
  "sources": ["string — which context sections were used"]
}
No markdown, no explanation, no code fences — raw JSON only.
```

Then validate the response against the schema. If it fails, send the validation error back and ask for correction (bounded retries — 2 is usually enough).

### Few-shot examples
Show, don't tell. One concrete example beats three paragraphs of instruction:
```
Input: "The server is slow"
Output: {"category": "performance", "severity": "medium", "action": "investigate"}
```

### Chain-of-thought for complex reasoning
When the task requires multi-step logic, ask the model to think before answering — but use a separate field so your parser can skip it:
```
{
  "reasoning": "your step-by-step analysis",
  "answer": "the final answer based on your reasoning"
}
```

## RAG (Retrieval-Augmented Generation)

### Architecture
```
User query
  → Embed the query (vector representation)
  → Search vector DB for similar chunks
  → Retrieve top-k relevant document chunks
  → Compose prompt: system + retrieved context + user question
  → Model generates answer grounded in the context
```

### Chunking strategy
| Method | Size | When |
|---|---|---|
| Fixed-size with overlap | 256-512 tokens, 50 overlap | Default; simple, works for most docs |
| Sentence boundaries | Variable | When cutting mid-sentence loses meaning |
| Section/heading | Variable | Structured docs (technical, legal) |
| Semantic | Embedding-based grouping | Best quality, most expensive |

**Rule**: chunks should be self-contained. A chunk that starts mid-argument or ends mid-sentence retrieves poorly.

### Embedding selection
- **OpenAI text-embedding-3-small**: good default, 1536 dimensions, cheap
- **OpenAI text-embedding-3-large**: better quality, 3072 dimensions, more expensive
- **Local (sentence-transformers)**: free, private, slower; all-MiniLM-L6-v2 is the baseline

### Retrieval quality
- Hybrid search (BM25 + vector) beats pure vector for keyword-heavy queries
- Rerank with a cross-encoder if you have >10 candidates
- Filter by metadata (date, source, category) before similarity search
- **Grounding**: include the source in the prompt so the model can cite it

## Common Pitfalls

### 1. Not handling the "I don't know" case
Models will confidently answer questions they have no information about. Instruct: "If the retrieved context doesn't contain the answer, say 'I don't have that information' — do not guess."

### 2. Token limit surprises on long conversations
Each message re-sends the full history. After 20 exchanges, your input tokens may exceed the model's context window. Solutions: summarize older turns (compaction), sliding window (keep last N), or extract key facts into a state object.

### 3. Inconsistent outputs across calls
Temperature 0 doesn't guarantee identical outputs (only near-deterministic). For exact reproducibility, cache the result and return the cache on repeat calls.

### 4. Streaming failures treated as total failures
If you get 80% of a response and the stream drops, don't throw it away. Return the partial with a "connection interrupted" flag. The user can regenerate.

### 5. Not logging token usage
LLM costs are invisible until they're not. Log input/output/cache tokens per request; alert on anomalous spikes; set per-user and per-feature budgets.

### 6. Prompt injection through retrieved content
If you RAG over user-submitted or external content, a malicious document can inject instructions. Sanitize: mark retrieved content as data (not instructions), instruct the model to treat context as informational only, and never execute tool calls triggered by retrieved content without validation.

## Definition of Done

- [ ] All LLM calls have retry, timeout, and rate limiting
- [ ] Streaming connections handle partial responses and drops
- [ ] Model outputs are validated against expected schemas before use
- [ ] Token usage is logged and monitored per feature
- [ ] Prompt injection via retrieved/user content is mitigated
- [ ] "I don't know" is an acceptable and handled response
