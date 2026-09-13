---
name: typescript
description: Load when writing or reviewing TypeScript to apply narrowing, generics, utility and mapped types, strict tsconfig settings, discriminated-union state machines, and correct any/unknown/never usage.
---

# TypeScript

Make illegal states unrepresentable: prefer unions whose members carry
exactly the data they need over widened types plus runtime checks. Types
are documentation the compiler verifies.

## Type Narrowing

```ts
function format(x: string | number): string {
  if (typeof x === "string") return x.trim(); // narrowed to string
  return x.toFixed(2);                        // narrowed to number
}
if ("email" in contact) contact.email;        // key presence
if (err instanceof HttpError) err.status;     // prototype
```

Discriminated unions are the workhorse: a literal `kind` field lets
`switch` narrow each branch at zero runtime cost.

```ts
type Shape =
  | { kind: "circle"; r: number }
  | { kind: "rect"; w: number; h: number };

function area(s: Shape): number {
  switch (s.kind) {
    case "circle": return Math.PI * s.r ** 2;
    case "rect":   return s.w * s.h;
    default: { const bad: never = s; return bad; } // exhaustiveness
  }
}
```

Use type predicates for reusable guards and assertion functions to fail
loudly at boundaries:

```ts
function isFish(pet: Fish | Bird): pet is Fish {
  return "swim" in pet;
}
function assertDefined<T>(v: T | undefined): asserts v is T {
  if (v === undefined) throw new Error("expected defined value");
}
```

## Generics

```ts
function prop<T, K extends keyof T>(obj: T, key: K): T[K] { // constraint
  return obj[key];
}
type Unwrap<T> = T extends Promise<infer U> ? U : T; // infer extracts U
type Elem<T> = T extends (infer E)[] ? E : never;
interface Store<T = unknown> { value: T }            // default param
```

- Constrain with `extends` only as tightly as needed: `<T extends
  { id: string }>` instead of `<T extends Item>` stays open to
  structurally compatible inputs.
- Use `infer` inside conditional types to pull types apart instead of
  hand-maintaining parallel hierarchies.
- Use `NoInfer<T>` to pin a parameter to another argument's type.

## Utility Types

| Utility | Produces | Typical use |
|---------|----------|-------------|
| `Partial<T>` | All props optional | Patch/update payloads |
| `Required<T>` | All props required | Final construction step |
| `Pick<T, K>` | Subset of keys | Public view of an internal type |
| `Omit<T, K>` | Keys removed | Hiding implementation fields |
| `Record<K, V>` | Object with keys K | Lookup tables over a literal union |
| `ReturnType<F>` | Return type of F | Deriving a type from a function |
| `Parameters<F>` | Parameter tuple of F | Forwarding wrappers |
| `Awaited<T>` | Unwrapped promise type | `Awaited<ReturnType<api>>` |
| `NoInfer<T>` | Blocks an inference site | Pinning a param to its declared type |

Reach for built-ins before writing mapped machinery by hand; compose
two (`Omit<Pick<T, K>, "internal">`) before writing a custom type.

## Mapped Types

```ts
type Nullable<T> = { [K in keyof T]: T[K] | null };
type Getters<T> = {
  [K in keyof T as `get${Capitalize<K & string>}`]: () => T[K] // remap
};
```

`keyof T` yields the key union; `T[K]` is indexed access. Remap with
`as` to rename or filter keys in one pass (`as never` drops a key).

## Template Literal Types

```ts
type Domain = "user" | "order";
type Event = `${Domain}${"Created" | "Updated" | "Deleted"}`;
// "userCreated" | ... | "orderDeleted"
function on<E extends Event>(event: E, handler: (e: E) => void) {}
```

Derive unions mechanically (event names, route params, config keys)
instead of maintaining parallel literals by hand.

## tsconfig Strict Mode

Enable `strict: true`; every flag it turns on catches a real bug class.

| Flag | Catches |
|------|---------|
| `noImplicitAny` | Untyped params silently spreading `any` |
| `strictNullChecks` | `undefined`/`null` flowing into dereferences (the big one) |
| `strictFunctionTypes` | Unsafe parameter contravariance in function types |
| `strictBindCallApply` | Wrong arity/types with `bind`/`call`/`apply` |
| `strictPropertyInitialization` | Declared-but-never-set class fields |
| `noImplicitThis` | `this` used where its type is unknown |
| `useUnknownInCatchVariables` | `catch (e)` typed `unknown`, not `any` |

Also enable `noUncheckedIndexedAccess` (so `arr[i]` is `T | undefined`)
on codebases that parse external input. Never disable strict flags
file-by-file: the flag failure is the finding.

## Declaration Files

```ts
// ambient module for an untyped dependency (types/*.d.ts)
declare module "legacy-lib" {
  export function init(opts: { debug?: boolean }): void;
}
declare global {
  interface Window { analytics?: { track(name: string): void } }
}
export {}; // keeps the file a module
// module augmentation — extend a library's types, don't fork them
declare module "express-serve-static-core" {
  interface Request { user?: SessionUser }
}
```

Prefer a minimal hand-written `.d.ts` over `// @ts-ignore` at every
import site. Declare what you have verified, not aspirational API.

## Discriminated Unions for State Machines

Model async and form states as unions so each state carries exactly its
data — "loading with data" stops being a boolean pile.

```ts
type Request<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: T }
  | { status: "error"; error: Error };

type Result<T, E> =
  | { ok: true; value: T }
  | { ok: false; error: E };

// Each switch/if branch carries only the fields that state owns:
// loading has no data, error has no data, success has no error.
// Impossible states do not compile.
```

## any vs unknown vs never

| Type | Meaning | Policy |
|------|---------|--------|
| `unknown` | Anything; must be narrowed before use | External input, catch vars, `JSON.parse` |
| `any` | Opt out of checking | Last resort behind a validated boundary; never in signatures |
| `never` | No possible value | Exhaustiveness checks, impossible branches, always-throw returns |

```ts
function parse(raw: unknown): Config {  // accept unknown, return known
  if (isConfig(raw)) return raw;
  throw new Error("invalid config");
}
```

Prefer `as const` over `as T` assertions: the former derives truth from
a value; the latter asserts wishes about it.

## Common Pitfalls

- Excess property checks run only on object literals: assigning through
  a variable skips the error. Keep initialization literal.
- Method syntax is bivariant; arrow-property syntax is checked strictly
  under `strictFunctionTypes`. Prefer `onChange: (v: string) => void`
  in callback interfaces.
- Structural typing accepts extra properties: identical shapes are
  interchangeable. Add a brand (`__brand: "UserId"`) when identity
  matters.
- Enums: prefer literal unions (`"asc" | "desc"`); `const enum` breaks
  `isolatedModules` setups; for a runtime object use `as const` plus
  `typeof`.
- `as` casts silence the compiler without checking: narrow or validate
  (zod) instead.
- Inference from `[]` or `{}` yields `never`/empty shapes: annotate the
  declaration, not the push site.
