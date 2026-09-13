---
name: react-patterns
description: Load when writing or reviewing React components to apply hooks correctly, choose state management tiers, manage effects and races, split code, place error boundaries, and test behavior.
---

# React Patterns

Apply when building React components or hooks. Order of defense: model
state correctly first, render derived values, and reach for effects and
memoization last — they patch modeling mistakes.

## Hooks Rules

Call hooks only at the top level of components or custom hooks, and
only unconditionally. React tracks hooks by call order per component
instance; a hook behind an `if` shifts the order on the next render and
corrupts every hook after it. Never disable the react-hooks lint rule;
restructure instead (early returns after all hooks, conditional logic
inside the hook).

## useState

```jsx
const [items, setItems] = useState(() => loadFromStorage()); // lazy init:
// expensive initial computation runs once, not on every render

setCount((prev) => prev + 1);      // functional update: immune to
setItems((prev) => [...prev, item]); // stale closures under batching
```

Use lazy initialization for any non-trivial initializer (`JSON.parse`,
localStorage, filters). Prefer functional updates whenever the next
value depends on the current one: under concurrent rendering, reading
state captured at render time is how stale bugs are born.

## useEffect

An effect synchronizes a component with a system outside React
(network, DOM, subscriptions, timers). If no external system is
involved, it probably does not belong in an effect.

```jsx
useEffect(() => {
  const ac = new AbortController();
  fetch(url, { signal: ac.signal })
    .then((r) => r.json())
    .then(setData)
    .catch((e) => { if (e.name !== "AbortError") setError(e); });
  return () => ac.abort();  // cleanup cancels the stale request
}, [url]);                  // re-run when url changes
```

- List every reactive value the effect reads in the dependency array.
  Lying about deps (eslint-disable) ships stale-data bugs.
- Return cleanup for anything cancellable: aborts, unsubscribes,
  intervals, observers.
- The race to guard: `url` changes and the first response lands after
  the second. Abort in cleanup (above) or ignore stale responses with
  an `let active` flag.
- Do NOT use an effect for: derived values (`full = first + last` —
  render them), event reactions (do it in the handler), resetting state
  on prop change (set a `key`), or chained state updates (one update or
  a reducer).

## useContext

Context is for low-frequency, app-wide values: theme, locale, current
user, feature flags. Every consumer re-renders when the value changes,
so never put rapidly changing state in one global context.

```jsx
const ThemeStateCtx = createContext(null);
const ThemeDispatchCtx = createContext(null);

function ThemeProvider({ children }) {
  const [theme, setTheme] = useState("light");
  return (
    <ThemeStateCtx.Provider value={theme}>
      <ThemeDispatchCtx.Provider value={setTheme}>
        {children}
      </ThemeDispatchCtx.Provider>
    </ThemeStateCtx.Provider>
  );
}
```

- Split state and dispatch contexts so dispatch-only consumers skip
  value-change re-renders.
- Memoize object provider values (`useMemo`); a fresh literal each
  render re-renders every consumer for nothing.
- Do not use context to avoid prop drilling that only two layers share:
  pass props or compose components.

## Custom Hooks

Extract repeated logic into hooks named `useXxx` — the units of
composition and the only legal place besides components to call hooks.

```jsx
function useFetch(url) {
  const [state, setState] = useState({ status: "loading" });
  useEffect(() => { /* abort logic above, calling setState */ }, [url]);
  return state;
}
```

Keep custom hooks return-stable: return memoized callbacks (or a
dispatch-style API) so callers' effect deps do not churn. One hook, one
concern — `useFetch` should not also format dates.

## Performance

| Tool | Actually helps when | Cargo cult when |
|------|--------------------|-----------------|
| `React.memo` | Same props, expensive render, re-rendered often by parent | Wrapping everything "for speed" |
| `useMemo` | Expensive computation, or identity used as a dep/prop | Memoizing cheap calculations |
| `useCallback` | Function identity passed to a memoized child or effect deps | Wrapping every handler |

Measure with the Profiler before optimizing. Code-split user-invisible
routes and widgets:

```jsx
const AdminPanel = React.lazy(() => import("./AdminPanel"));

<Suspense fallback={<Spinner />}>
  <AdminPanel />
</Suspense>
```

## State Management Decision

| Tier | Situation | Choose |
|------|-----------|--------|
| 1 | State local to one component | `useState` (inputs, open/closed) |
| 2 | Shared by nearby components | Lift to closest parent; context for low-frequency values |
| 3 | App-wide client state, high frequency | Zustand / Jotai (selector subscriptions) |
| 4 | Complex domain workflows, middleware, devtools, big teams | Redux Toolkit |

Server data (fetch/cache/invalidate) is a different problem: use a
query library (TanStack Query) instead of storing responses in Redux or
context. Climb tiers only when the current one measurably hurts.

## Forms

Use controlled inputs when values drive rendering (cross-field
validation, conditional UI); uncontrolled for large or perf-sensitive
forms. Reach for react-hook-form past a handful of fields:

```jsx
const { register, handleSubmit, formState: { errors } } = useForm({
  resolver: zodResolver(schema),  // schema is the single source of truth
});

<form onSubmit={handleSubmit(onSave)}>
  <input aria-invalid={!!errors.email} {...register("email")} />
  {errors.email && <p role="alert">{errors.email.message}</p>}
  <button>Save</button>
</form>
```

Share the validation schema with the server so both enforce the same
rules. Tie each error to its field (`aria-describedby`) so it is
announced — see the frontend-fundamentals skill.

## Error Boundaries

Boundaries are still class components; function components cannot catch
render errors. Place one per route or panel so one crash does not blank
the app; reset via `key` on navigation.

```jsx
class ErrorBoundary extends React.Component {
  state = { error: null };
  static getDerivedStateFromError(error) { return { error }; }
  componentDidCatch(error, info) { report(error, info); }
  render() {
    return this.state.error
      ? <Fallback error={this.state.error} />
      : this.props.children;
  }
}
```

Boundaries catch render and lifecycle errors only — not event handlers
(wrap those in try/catch), not async callbacks, not server errors.

## Concurrent Features

```jsx
const [isPending, startTransition] = useTransition();
const deferredQuery = useDeferredValue(query);

startTransition(() => setResults(filter(huge, query))); // non-urgent:
// typing stays smooth instead of blocking on the expensive update
```

Mark updates non-urgent when input must stay responsive while derived
work is heavy (search-as-you-type, typeahead, tab switches).
`useDeferredValue` does the same for values whose setter you do not
control.

## Testing

Test behavior as the user experiences it: render, interact, assert on
output. Never assert internal state or private structure — those tests
break on every refactor and catch nothing.

```jsx
test("shows validation error for bad email", async () => {
  render(<Signup />);
  await userEvent.type(
    screen.getByRole("textbox", { name: /email/i }), "nope");
  await userEvent.click(screen.getByRole("button", { name: /sign up/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/valid/i);
});
```

Query priority: `getByRole` (mirrors what assistive tech sees, and
fails when markup is inaccessible) > `getByLabelText` >
`getByPlaceholderText` > `getByText` > `getByTestId` (last resort).
Use `userEvent` over `fireEvent` for real interaction semantics;
use `find*` for elements that appear asynchronously.

## Common Pitfalls

- Effect without a dep array that sets state: infinite render loop.
- Effect to derive state: render it directly or `useMemo` it instead.
- Stale closure reading state captured at render time: use functional
  updates, or a ref for the truly-latest value.
- Array index as `key` on reorderable lists: corrupts focus and
  component state; key by stable id.
- Context value object recreated every render: memoize or split it.
- `setState` during render (outside events/effects): move to an event
  handler or the `key`-reset pattern.
- Fetch without abort: races paint old responses over new ones.
- Subscriptions in an effect without cleanup: leaks plus
  setState-after-unmount warnings.
