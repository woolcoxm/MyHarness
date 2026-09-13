---
name: python-craft
description: Use whenever writing, reviewing, or optimizing Python — typing with Protocol and TypeVar, dataclass design, asyncio with TaskGroup, pytest fixtures and parametrize, pyproject.toml packaging with uv or poetry, or profiling with cProfile and line_profiler. Covers comprehension idioms, EAFP error handling, GIL-aware concurrency choices, and classic pitfalls like mutable default arguments and late-binding closures so code passes strict mypy/pyright on the first run.
---

# Python: Idiomatic, Typed, and Tested

Write Python that a strict type checker and a senior reviewer both accept: comprehensions over loops, protocols over inheritance, explicit errors, and fast tests. Modern Python (3.10+) is statically checkable — treat it that way from the first commit.

## Idioms

```python
# Comprehensions with conditions; generator expression for big data
evens = [x for x in nums if x % 2 == 0]
total = sum(x * x for x in nums)          # no []: lazy, O(1) memory
uniq = {name.strip().lower() for name in names}

# Unpacking and multiple assignment
first, *rest = items
a, b = b, a                               # swap
for i, (key, val) in enumerate(pairs): ...

# Walrus operator: assign inside a condition
while (chunk := stream.read(8192)):
    process(chunk)
```

WHY generator expressions: a list comprehension over 10M rows materializes 10M references up front; a genexp holds one item at a time.

## Typing

Run mypy or pyright in strict mode from day one — retrofitting types onto a grown codebase costs an order of magnitude more. Use modern syntax (`int | None`, not `Optional[int]`).

```python
from typing import Protocol, TypeVar, TypedDict, Literal

class Closeable(Protocol):                # structural typing: duck-typed
    def close(self) -> None: ...          # any type with close() satisfies it

T = TypeVar("T")
def first(xs: list[T]) -> T: ...          # generic preserves element type

class Config(TypedDict):                  # dict with a checked shape
    retries: int
    mode: Literal["fast", "safe"]         # typo becomes a type error
```

Use `Protocol` when you cannot modify the implementing types (stdlib, third-party). Use `Literal` or enums for closed string sets so typos fail at type-check time, not in production.

## Dataclasses

```python
from dataclasses import dataclass, field

@dataclass(frozen=True, slots=True)
class Point:
    x: float
    y: float = 0.0
    tags: tuple[str, ...] = ()            # immutable default is safe

    def __post_init__(self) -> None:
        if self.x < 0:
            raise ValueError("x must be >= 0")
```

`frozen=True` gives hashable, immutable values — use it for config and value objects. `slots=True` cuts per-instance memory roughly in half for many small objects. Mutable defaults are rejected by design; use `field(default_factory=list)`. Reach for attrs or pydantic only when you need rich validation or serialization beyond what dataclasses provide.

## Async Python

Async helps I/O-bound work (network, disk, many sockets); it does nothing for CPU-bound code, which the GIL still serializes — use processes for that.

```python
async with asyncio.TaskGroup() as tg:     # 3.11+: structured concurrency
    results = [tg.create_task(fetch(u)) for u in urls]
# a failing task cancels its siblings — errors never get orphaned

async for line in stream: ...             # async generator consumption
async with session.post(url) as resp: ... # async context manager
```

`asyncio.gather` is fine for simple fan-out; pass `return_exceptions=True` consciously or one failure cancels the rest. Every `async def` should await something — a CPU-heavy coroutine blocks the event loop. Offload blocking calls with `asyncio.to_thread`.

## Error Handling

EAFP (easier to ask forgiveness than permission) is idiomatic: attempt the operation and handle the failure, instead of checking first and racing anyway.

```python
try:
    cfg = load(path)
except FileNotFoundError as e:
    raise ConfigError(f"missing config {path}") from e  # keep the cause
```

Define one exception hierarchy per package (`AppError` with specialized subclasses) so callers can catch precisely. `raise ... from e` preserves the traceback chain — never re-raise as `str(e)`. Clean up with context managers: `@contextmanager` for the simple case, `__enter__`/`__exit__` for stateful objects; the `__exit__` return value controls suppression.

## Packaging

`pyproject.toml` is the single source of truth for metadata, dependencies, and tool configuration — no `setup.py`, no split config files. Pick one resolver and stay consistent: uv (fastest, modern default), poetry (mature all-in-one, heavier), or pip-tools (plain pip plus compiled lockfiles, minimal). Always work inside a virtual environment, and pin transitive dependencies (uv.lock, poetry.lock, compiled requirements) for reproducible builds.

```toml
[project.scripts]
mytool = "mytool.cli:main"   # entry point: installed console command
```

Namespace packages (directories without `__init__.py`) exist to split one logical package across distributions — rarely what a normal project needs.

## Testing with pytest

```python
@pytest.fixture
def db(tmp_path):                        # tmp_path is a built-in fixture
    conn = Database(tmp_path / "t.db")
    yield conn                           # code after yield is teardown
    conn.close()

@pytest.mark.parametrize("raw,want", [
    ("1", 1), ("0x10", 16), ("-5", -5),
])
def test_parse(raw, want, db):
    assert parse(raw, db) == want

def test_retry(mocker):                  # or use monkeypatch.setattr
    mocker.patch("mytool.cli.load")
```

Put shared fixtures in `conftest.py` — auto-discovered, no imports needed. Register custom markers in pyproject to avoid warnings. Async tests need `pytest-asyncio` with `asyncio_mode = "auto"`. Prefer hand-written fakes over mock libraries; mock only at true external boundaries.

## Performance

Measure before optimizing: `python -m cProfile -s cumtime app.py` to find the hot spot, then `line_profiler` on that function. Then apply, in order of preference:

- Algorithm change (O(n²) → O(n log n)) beats any micro-optimization
- NumPy vectorization instead of Python loops (`np.add.reduce(xs)`)
- `@functools.lru_cache` on pure functions with repeated arguments
- `slots=True`/`__slots__` for many small instances
- Generators instead of materialized lists for large datasets

Build strings with `"".join(parts)`, not `+=` in a loop — repeated concatenation is O(n²) in the worst case.

## Common Pitfalls

- Mutable default arguments: `def f(x, acc=[])` — one list shared by every call. Use `acc: list | None = None`, then `acc = [] if acc is None else acc`.
- Late binding in closures: `[lambda: i for i in range(3)]` all return 2 — bind at definition time with `lambda i=i: i`.
- Integer vs float division: `3 / 2 == 1.5` but `3 // 2 == 1`, and `//` floors toward negative infinity (`-3 // 2 == -2`). Pick deliberately.
- GIL: threads help I/O-bound code and never CPU-bound code — use `multiprocessing` or native extensions for CPU work.
- `is` vs `==`: `is` compares identity, `==` compares value; only `is None` is idiomatic (small-int and string interning make `is` unreliable).

## Definition of Done

- [ ] mypy/pyright strict passes with zero errors
- [ ] No mutable default arguments or unbound closure captures
- [ ] Public functions fully type-hinted; exceptions chained with `from`
- [ ] pytest suite green, parametrized where inputs vary, fixtures in conftest
- [ ] pyproject.toml is the single config source; dependencies pinned
- [ ] Hot paths measured with cProfile before and after any optimization
