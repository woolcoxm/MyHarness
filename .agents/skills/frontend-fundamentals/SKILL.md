---
name: frontend-fundamentals
description: Load when creating or reviewing any HTML page, template, or web UI to apply semantic structure, accessibility, form UX, SEO metadata, Core Web Vitals budgets, and progressive enhancement.
---

# Frontend Fundamentals

Apply these rules when writing or reviewing HTML for pages, components, or
templates. Work in this order: structure (semantic HTML), people
(accessibility), interaction (forms), discovery (SEO), speed (vitals),
resilience (progressive enhancement).

## Semantic HTML

Pick elements for meaning, not appearance. Assistive tech and crawlers
consume the document outline; `div` soup destroys that signal, and ARIA
cannot reliably restore it.

| Element | Means | Use for | Avoid for |
|---------|-------|---------|-----------|
| `nav` | Major navigation | Site menu, breadcrumbs, TOC | Any list of links |
| `main` | Dominant content, one per page | Unique content region | Wrapping the whole page |
| `article` | Self-contained, syndicable unit | Blog post, product card, comment | Arbitrary text block |
| `section` | Thematic group with a heading | Chapter, tab panel, cluster | Generic divider (use `div`) |
| `aside` | Tangential content | Sidebar, pull quote, related links | Layout column (use grid/flex) |
| `footer` | Closing matter of its section | Page footer, byline block | Anything merely at the bottom |

```html
<body>
  <nav aria-label="Primary"><!-- site links --></nav>
  <main>
    <article>
      <h1>Post title</h1>
      <section aria-labelledby="comments-h">
        <h2 id="comments-h">Comments</h2>
      </section>
    </article>
    <aside><h2>Related reading</h2></aside>
  </main>
  <footer><!-- colophon --></footer>
</body>
```

Use exactly one `<h1>` per page; never skip heading levels for visual
reasons — restyle size with CSS. Headings alone must read like a table
of contents.

## Accessibility Essentials

### Keyboard navigation

- Build interactive elements from native `button`, `a`, `input`,
  `select`, `textarea`: focus, key handling, and roles come free.
- If a custom widget is unavoidable: `tabindex="0"`, the correct
  `role`, Enter and Space handlers. Never use positive `tabindex`.
- Tab through every flow before shipping; each control must be
  reachable and operable without a mouse.

### Focus management

- Modals and drawers: move focus in on open, trap Tab inside, restore
  focus to the trigger on close.
- SPA route changes: move focus to `main` or the new `h1` so screen
  readers announce the navigation.
- Style focus with `:focus-visible`; never remove outlines without a
  visible replacement.

### ARIA roles

First rule of ARIA: use a native element instead when one exists. A
`<button>` beats `<div role="button">`, which still needs tabindex,
keys, and states wired by hand.

```html
<!-- Bad: not focusable, no role, no keys -->
<div onclick="save()">Save</div>
<button type="button" onclick="save()">Save</button> <!-- Good -->
```

Prefer labelling by visible content: `aria-labelledby` first, then
`aria-label`; attach hints and errors with `aria-describedby`.

### Color contrast

| Content | Minimum ratio |
|---------|---------------|
| Body text (under 24px, or under 18.7px bold) | 4.5:1 |
| Large text (24px+, or 18.7px+ bold) | 3:1 |
| UI components and meaningful graphics | 3:1 |

Check every state (hover, focus, visited) and every color scheme with a
contrast checker. Do not eyeball it.

### Screen-reader testing

Run a two-minute pass before shipping a flow: navigate by headings (H),
lists (L), links (K) with NVDA (Windows), VoiceOver (macOS), or
Narrator. Confirm each control announces name, role, and value.

## Form Design

Associate every control with a visible `<label for>`; `placeholder` is
a hint, never a label. Validate on blur and on submit, not per
keystroke. Tie each error to its input and say exactly what to fix.

```html
<form action="/signup" method="post" novalidate>
  <p>
    <label for="email">Email address</label>
    <input id="email" name="email" type="email"
           autocomplete="email" inputmode="email"
           aria-describedby="email-error" required>
    <span id="email-error" role="alert" hidden>Enter a valid address.</span>
  </p>
  <button type="submit">Create account</button>
</form>
```

- Keep errors persistent until fixed; do not clear what the user typed.
- Fill `autocomplete` with the correct token so browsers and password
  managers help: `given-name`, `family-name`, `email`, `tel`,
  `postal-code`, `cc-number`, `one-time-code`, `new-password`,
  `current-password`.
- Put the submit button inside the form; `type="submit"`, never a div.
- On failed submit of a long form, show an error summary with links
  that focus each invalid field.

## SEO Fundamentals

Give every indexable page a unique title, meta description, and one
canonical URL.

```html
<title>Buy running shoes - Acme Sports</title>
<meta name="description" content="Road and trail shoes, free returns.">
<link rel="canonical" href="https://acme.example/shoes/running">
<meta property="og:title" content="Running shoes - Acme Sports">
<meta property="og:image" content="https://acme.example/og/run.png">
<meta property="og:url" content="https://acme.example/shoes/running">
<meta name="twitter:card" content="summary_large_image">
```

Describe the page's primary entity with JSON-LD structured data.

```html
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "Product",
  "name": "Acme Road Runner",
  "image": ["https://acme.example/img/runner-1200.jpg"],
  "offers": { "@type": "Offer", "price": "119.99", "priceCurrency": "USD" }
}
</script>
```

- Keep titles ~50-60 chars and descriptions ~150-160 chars, unique per
  page, primary keyword first, written for clicks.
- Emit `rel="canonical"` when content is reachable via multiple URLs
  (query params, sort orders) to consolidate ranking signals.

## Core Web Vitals

| Metric | Measures | Good | Improve by |
|--------|----------|------|------------|
| LCP | Largest Contentful Paint (perceived load) | 2.5 s or less | Preload the hero image (`fetchpriority="high"`), `font-display: swap`, inline critical CSS, drop render-blocking scripts, CDN |
| CLS | Cumulative Layout Shift (visual stability) | 0.1 or less | Set `width`/`height` or `aspect-ratio` on media, reserve space for ads/embeds, never insert content above visible content |
| INP | Interaction to Next Paint (responsiveness) | 200 ms or less | Break up long tasks, avoid layout thrash in handlers, defer non-critical JS, yield with `scheduler.yield()` |

Measure in the field (CrUX, RUM), not only Lighthouse. For interactive
apps, fix INP first; for content pages, fix LCP first.

## Progressive Enhancement

Layer the build so each tier degrades to the one beneath it:

1. HTML carries content and action: real URLs, a real form `action`,
   server-side handling that works with scripting disabled.
2. CSS adds layout and visual design.
3. JS enhances: AJAX submission, inline validation, streaming updates.

```js
form.addEventListener("submit", async (event) => {
  if (!form.checkValidity()) return;  // native path still works
  event.preventDefault();             // now enhance
  await fetch(form.action, { method: "POST", body: new FormData(form) });
});
```

Test each critical flow once with JS disabled. A failed script bundle
must never blank the page or block checkout.

## Common Pitfalls

- `div` with `onclick` where a `button` belongs: no focus, keys, or role.
- `placeholder` as the only label: vanishes on input, low contrast.
- Icon-only buttons without an accessible name: add `aria-label`.
- Images without `width`/`height`: every lazy-load becomes layout shift.
- An ARIA role without its keyboard handlers: worse than native.
- Modals that skip focus restoration: keyboard users land at page top.
- Heading levels chosen for font size: breaks the outline; use CSS.
- JS-only navigation that breaks middle-click and the back button.
