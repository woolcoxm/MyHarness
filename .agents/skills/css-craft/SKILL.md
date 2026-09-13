---
name: css-craft
description: Load when writing or refactoring stylesheets to choose flexbox vs grid, responsive strategy, custom-property theming, and 60fps animation, and to dodge cascade and stacking traps.
---

# CSS Craft

Reach for this skill whenever writing layout, theming, or animation CSS.
Default to the least-powerful tool that works: flow, then flexbox, then
grid, then JS measurement.

## Flexbox Mental Model

Flexbox distributes space along one axis. `flex-direction` sets the main
axis; the perpendicular is the cross axis. `justify-*` always means main
axis, `align-*` always means cross axis, whatever the direction.

```css
.toolbar {
  display: flex;
  flex-direction: row;            /* main axis = horizontal */
  justify-content: space-between; /* positions along MAIN axis */
  align-items: center;            /* sizes/positions on CROSS axis */
  gap: 0.75rem;                   /* space between items, not margins */
}
```

- Use `gap`, not margins, between siblings: no first/last-child cleanup.
- Wrapping creates flex lines; only then does `align-content` act (it
  spaces lines, not items within them).
- Write `flex: 1 1 0` (grow, shrink, basis) rather than `flex: 1` for
  equal-width columns: basis `0` splits space equally, `auto` sizes
  from content.
- `flex-wrap: wrap` plus `flex-basis` on children handles tag/chip rows.

## CSS Grid

Grid places items in two dimensions at once — use it when rows and
columns must align: page shells, galleries, forms.

```css
.page {
  display: grid;
  grid-template-columns: 240px minmax(0, 1fr);
  grid-template-rows: auto 1fr auto;
  grid-template-areas:
    "side header"
    "side main"
    "side footer";
  min-height: 100vh;
}
.side { grid-area: side; } /* place children into named areas */
```

Always pair flexible columns with `minmax(0, 1fr)`, never bare `1fr`:
plain `1fr` has an implicit minimum of `auto`, so long unbreakable
content (URLs, tables) blows out the track.

| Function | Behavior | Use when |
|----------|----------|----------|
| `repeat(auto-fit, minmax(180px, 1fr))` | Collapses empty tracks; items stretch to fill the row | Card grid that should look full at every width |
| `repeat(auto-fill, minmax(180px, 1fr))` | Keeps empty tracks; grid keeps its rhythm | Wrapped grids where alignment beats fullness |

## Responsive Design

Write mobile-first: base styles target the small screen, then layer
`min-width` queries upward. Designing down from desktop breeds
override wars.

```css
.card-grid { display: grid; gap: 1rem; grid-template-columns: 1fr; }
@media (min-width: 40rem) { .card-grid { grid-template-columns: repeat(2, 1fr); } }
@media (min-width: 64rem) { .card-grid { grid-template-columns: repeat(4, 1fr); } }
```

| Choose | When the breakpoint depends on | Example |
|--------|-------------------------------|---------|
| Container query | The component's own box (reusable widgets, sidebars) | Card collapses when its column is narrow |
| Media query | The viewport (nav layout, page margins) | Drawer nav becomes top nav |

```css
.card { container-type: inline-size; }
@container (min-width: 30rem) {
  .card__body { display: grid; grid-template-columns: 1fr 2fr; }
}
```

Fluid typography with `clamp()` removes breakpoints; clamp with a rem
minimum so user font-size preferences still apply.

```css
h1 { font-size: clamp(1.75rem, 1rem + 3vw, 3rem); }
```

## Custom Properties

Define theme tokens once; theming becomes swapping token values instead
of hunting hex codes.

```css
:root { --surface: #fff; --text: #1a1c20; --accent: #0b62d6; }
@media (prefers-color-scheme: dark) {
  :root { --surface: #14161a; --text: #e8eaed; }
}
[data-theme="dark"] { --surface: #14161a; --text: #e8eaed; }

body { background: var(--surface); color: var(--text); }
button { color: var(--accent); }
```

- Prefer semantic tokens (`--surface`, `--text-muted`) over literal
  ones (`--blue-500`) so themes survive palette changes.
- Scope component knobs to the component (`.card { --gap: 1rem; }`) so
  consumers can retune it from outside.
- Custom properties cascade, inherit, and resolve at use — not Sass
  variables.

## Modern Selectors

```css
:is(h1, h2, h3) > a { color: inherit; }    /* specificity = highest arg */
:where(ul, ol) li { margin-block: 0.25em; } /* specificity = 0 */
.card:has(img) { padding-top: 0; }          /* style parent by children */
.card {
  & .title { font-weight: 600; }            /* native nesting */
  &:hover { translate: 0 -2px; }
}
```

Use `:where()` in resets and library styles so authors can override
without specificity fights. Use `:has()` for "contains" styling that
used to need JS class toggling.

## Transitions and Animations

Animate only `transform` and `opacity` for 60fps: the compositor
handles them without layout or paint. `width`, `height`, `top`, and
`left` trigger layout every frame.

```css
.card { transition: transform 200ms ease, opacity 200ms ease; }
.card:hover { transform: translateY(-4px) scale(1.01); }

@keyframes slide-in {
  from { opacity: 0; transform: translateY(8px); }
  to   { opacity: 1; transform: none; }
}
.drawer { animation: slide-in 250ms ease-out; }
```

Add `will-change: transform` only to fix measured jank, then remove it:
promoted layers cost memory. Always honor motion preferences:

```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
```

## Layout Patterns

- Holy grail: the named-area `.page` grid above; `auto 1fr auto` rows
  plus `min-height: 100vh` give a sticky footer for free.
- Sticky footer without grid: `body { min-height: 100vh; display: flex;
  flex-direction: column; } main { flex: 1; }`.
- Sidebar + content: `grid-template-columns: minmax(220px, 1fr) minmax(0, 3fr)`.
- Card grid: `repeat(auto-fit, minmax(min(100%, 240px), 1fr))` — the
  `min(100%, ...)` guard prevents overflow on narrow screens.
- Masonry: multicol (`columns: 3;` plus `break-inside: avoid;` on
  items). DOM order runs down columns; if reading order across rows
  matters, use grid row spans or JS layout.

## Specificity and the Cascade

Specificity counts (inline) > ids > classes, attributes, pseudo-classes
> elements: score `#id .class attr` as `1-0-0`, `.class.class` as
`0-2-0`. Avoid ids in CSS entirely — nothing overrides them but
`!important`.

Treat `!important` as a bug report: the cascade was arranged wrongly.
Restructure selectors, or use cascade layers (`@layer reset, base,
components, utilities;`) where later layers win regardless of selector
specificity — the tool that tames third-party CSS.

## Common Pitfalls

- Margin collapse: vertical margins between siblings and parents merge
  into the larger one; flex/grid containers and padding block it.
- Stacking contexts: `z-index: 9999` loses to an ancestor context.
  `opacity < 1`, `transform`, `filter`, `isolation: isolate` each
  create a context; fix the tree, not the number.
- Percentage of what? `width: 50%` resolves against the containing
  block's width; `height: 50%` needs a sized parent; vertical margins
  and paddings resolve against the *width* (`padding-top: 56.25%`).
- `inline-block` whitespace: markup newlines render as a 4px gap; use
  flex or `font-size: 0` on the parent.
- Bare `1fr` tracks overflow on long content: use `minmax(0, 1fr)`.
- `100vh` is taller than mobile Safari's viewport: use `100dvh`.
- Transitions to `height: auto` do not animate: animate `transform`
  or `grid-template-rows: 0fr -> 1fr` instead.
