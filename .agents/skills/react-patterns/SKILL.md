---
name: react-patterns
description: Load when writing or reviewing React components to apply hooks correctly, choose state management tiers, manage effects and races, split code, place error boundaries, and test behavior.
---

# React Patterns

Model state correctly first, render derived values, and reach for
effects and memoization last — they patch modeling mistakes.

## Hooks Rules

React tracks hooks by call order per component instance; a hook behind
an `if` corrupts every hook after it. Call hooks only at the top level,
unconditionally; never disable the lint rule — restructure.

## useState

```jsx
const [items, setItems] = useState(() => loadFromStorage()); // lazy init
setCount((prev) => prev + 1);        // functional update: immune to
setItems((prev) => [...prev, item]); // stale closures under batching
```

Use lazy initialization for non-trivial initializers (`JSON.parse`, localStorage).
Prefer functional updates when the next value depends on the current.

## useEffect

An effect synchronizes a component with an external system (network, DOM,
subscriptions, timers); anything else does not belong in an effect.

```jsx
useEffect(() => {
  const ac = new AbortController();
  fetch(url, { signal: ac.signal })
    .then((r) => r.json()).then(setData)
    .catch((e) => { if (e.name !== "AbortError") setError(e); });
  return () => ac.abort();  // cleanup cancels the stale request
}, [url]);                  // re-run when url changes
```

- List every reactive value the effect reads as a dep; lying about deps
  (eslint-disable) ships stale-data bugs.
- Return cleanup for anything cancellable: aborts, unsubscribes,
  intervals.
- The race to guard: `url` changes and the first response lands after
  the second — abort in cleanup or check an `active` flag.
- Do NOT use an effect for derived values (render them), event
  reactions (use the handler), resetting on prop change (set a `key`),
  or chained updates (fold into one).

## useContext

Context is for low-frequency, app-wide values (theme, locale, user).
Every consumer re-renders on value change — never put high-frequency
state in it.

```jsx
const ThemeState = createContext(null);
const ThemeDispatch = createContext(null);
function ThemeProvider({ children }) {
  const [theme, setTheme] = useState("light");
  return (
    <ThemeDispatch.Provider value={setTheme}>
      <ThemeState.Provider value={theme}>{children}</ThemeState.Provider>
    </ThemeDispatch.Provider>
  );
}
```

- Split state and dispatch contexts so dispatch-only consumers skip
  value-change re-renders.
- Memoize object provider values (`useMemo`); a fresh literal each
  render re-renders every consumer for nothing.
- Do not use context to dodge two-layer prop drilling: pass props.

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

Keep custom hooks return-stable: memoized callbacks (or a dispatch-style
API) so callers' effect deps do not churn. One hook, one concern.

## Performance

| Tool | Actually helps when | Cargo cult when |
|------|--------------------|-----------------|
| `React.memo` | Same props, expensive render, re-rendered often by parent | Wrapping everything "for speed" |
| `useMemo` | Expensive computation, or identity used as a dep/prop | Memoizing cheap calculations |
| `useCallback` | Function identity passed to a memoized child or effect deps | Wrapping every handler |

Measure with the Profiler before optimizing; code-split user-invisible
routes and widgets:

```jsx
const AdminPanel = React.lazy(() => import("./AdminPanel"));
<Suspense fallback={<Spinner />}><AdminPanel /></Suspense>
```

## State Management Decision

| Tier | Situation | Choose |
|------|-----------|--------|
| 1 | State local to one component | `useState` (inputs, open/closed) |
| 2 | Shared by nearby components | Lift to closest parent; context for low-frequency values |
| 3 | App-wide client state, high frequency | Zustand / Jotai (selector subscriptions) |
| 4 | Complex domain workflows, middleware, devtools, big teams | Redux Toolkit |

Server data (fetch/cache/invalidate) is a different problem: use TanStack
Query, not Redux or context. Climb tiers only when one measurably hurts.

## Forms

Use controlled inputs when values drive rendering (cross-field
validation, conditional UI); uncontrolled for large or perf-sensitive
forms:

```jsx
const { register, handleSubmit, formState: { errors } } = useForm({
  resolver: zodResolver(schema),  // single source of truth
});
<form onSubmit={handleSubmit(onSave)}>
  <input aria-invalid={!!errors.email} {...register("email")} />
  {errors.email && <p role="alert">{errors.email.message}</p>}
  <button>Save</button>
</form>
```

Share the validation schema with the server; tie each error to its field
(`aria-describedby`) — see the frontend-fundamentals skill.

## Error Boundaries

Boundaries are still class components; functions cannot catch render errors.
Place one per route or panel; reset via `key` on navigation.

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

Boundaries catch render/lifecycle errors only — not handlers (try/catch
those), async callbacks, or server errors.

## Concurrent Features

```jsx
const [isPending, startTransition] = useTransition();
const deferredQuery = useDeferredValue(query);
startTransition(() => setResults(filter(huge, query))); // non-urgent:
// typing stays smooth instead of blocking on the expensive update
```

Mark updates non-urgent when input must stay responsive during heavy derived
work (typeahead, tabs); `useDeferredValue` covers setters you don't control.

## Testing

Test behavior as the user experiences it: render, interact, assert on
output. Never assert internal state or private structure — those tests
break on every refactor and catch nothing.

```jsx
test("shows validation error for bad email", async () => {
  render(<Signup />);
  await userEvent.type(screen.getByRole("textbox", { name: /email/i }), "x");
  await userEvent.click(screen.getByRole("button", { name: /sign up/i }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/valid/i);
});
```

Query priority: `getByRole` (what assistive tech sees; fails on inaccessible
markup) > `getByLabelText` > `getByPlaceholderText` > `getByText` >
`getByTestId` (last resort). Use `userEvent`; use `find*` for async.

## Common Pitfalls

- Effect without a dep array that sets state: infinite render loop.
- Effect to derive state: render it directly or `useMemo` it instead.
- Stale closure reading captured state: functional updates, or a ref.
- Array index as `key` on reorderable lists corrupts focus; key by id.
- Context value object recreated every render: memoize or split it.
- `setState` during render: move to an event or the `key`-reset pattern.
- Fetch without abort: races paint old responses over new ones.
- Effect subscription without cleanup: leaks plus setState-after-unmount warnings.
