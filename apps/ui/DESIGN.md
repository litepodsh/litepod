# Design System: Litepod Product UI

## 1. Visual Theme & Atmosphere

Signal Oxide is a calm, technical interface for people who operate
infrastructure. It has daily-app density, measured asymmetric emphasis, and
restrained tactile motion. Product evidence and hierarchy do the visual work;
there are no gradients, neon effects, decorative grids, or marketing-page
treatments inside the application.

- Variance: 5/10. The shell can be offset, while operational screens stay easy
  to scan.
- Motion: 3/10. Hover, press, menu, and sidebar changes only.
- Density: 6/10. Compact enough for operations, never cramped.

## 2. Color Palette & Roles

| Name | Light | Dark | Role |
| --- | --- | --- | --- |
| Canvas | `#F4F0E9` | `#101512` | Application background |
| Surface | `#FFFDF8` | `#17201A` | Inputs, menus, dialogs, raised content |
| Recess | `#EBE7DF` | `#0C110E` | Important outer bezels and terminal wells |
| Ink | `#1A2520` | `#F2F0E9` | Primary text and strong actions |
| Muted | `#53635B` | `#B7C0B8` | Supporting text and quiet actions |
| Signal Oxide | `#D84B2A` | `#F06A48` | One primary action, focus and meaningful state |
| Accent Ink | `#FFFAF3` | `#20100B` | Text on Signal Oxide |

Use Signal Oxide for one dominant action per view. Never use it for long body
copy on a light surface. Do not add another bright accent.

### Status ramp

State is not decoration, so it gets its own muted ramp rather than borrowing the
accent. Warn is a cool brass, deliberately darker and less saturated than Signal
Oxide, so a pending badge never reads as a primary action.

| Name | Light | Dark | Role |
| --- | --- | --- | --- |
| OK | `#2F6B4F` | `#5FBF8F` | Running, delivered, healthy, succeeded |
| Warn | `#8A6410` | `#D9A441` | Pending, deploying, paused, degraded |
| Info | `#2B5F6B` | `#6FB6C4` | Ready, informational, build stages |
| Danger | `--destructive` | `--destructive` | Failed, dead, unknown container state |
| Neutral | Muted | Muted | Stopped, not deployed, unavailable |

Never write a raw Tailwind palette class (`bg-emerald-500`, `text-amber-700`)
in a component; those do not follow the theme and each site drifts. Map your
feature's status enum to a `StatusTone` and ask
`$lib/utils/status-tone.ts` for classes: `toneClass` (chip), `toneDot` (dot),
`toneBadge` (border + fill + text), `toneBorder` (outline), `toneFill` (a raw
value for canvas and SVG), `utilizationTone` (the shared 75/90 thresholds).
Container and application statuses already have a mapping in
`$lib/utils/container-status.ts`.

### Relief

`--lp-bevel` and `--lp-emboss` are one overhead lamp: a light top edge and a
shaded bottom one. In light theme the highlight carries it; in dark the two swap
roles and the shade does the work. Four utilities, declared with `@utility` in
`app.css` so they sit in Tailwind's utilities layer:

| Utility | Use for |
| --- | --- |
| `.lp-raised` | Anything sitting on the page: small chips, list rows, quiet buttons |
| `.lp-inset` | Wells that recede: inputs, selects, textareas, terminal grounds, the outer `Surface` bezel |
| `.lp-lifted` | Cards. Adds a wide canvas-tinted drop on top of the bevel |
| `.lp-shadow-room` | A clipping parent that holds cards, to keep their shadows from being cut |

Two rules that are easy to get wrong:

- **Never declare these unlayered.** Unlayered CSS outranks every Tailwind
  layer, so a raw `box-shadow` silently beats `ring-*` and `shadow-*` (rings are
  box-shadow in v4) and kills focus rings.
- **`.lp-lifted` stays under about `1.25rem` of reach.** Cards live in grids and
  grids live in scroll containers. Any parent with `overflow-hidden` (a
  collapsible) or `overflow-y-auto` (a scroll region, where the x axis is forced
  to clip too) cuts the shadow off. Give that parent `.lp-shadow-room`, and grow
  an explicit `max-h` by the `2.25rem` it adds. Dialogs want a bigger pool of
  shadow and set their own.

This is soft skeuomorphism, not neumorphism. Never match a control's fill to
its own background to make relief carry the boundary; contrast has to survive
with the shadows switched off.

## 3. Typography Rules

- Sans: Geist, then Satoshi, then system sans. Never Inter.
- Mono: Commit Mono for commands, timestamps, IDs, compact labels, and values
  that benefit from exact visual alignment.
- Headings: tight tracking (`-0.04em` to `-0.07em`), medium weight, controlled
  scale.
- Body: muted ink, relaxed line-height, maximum 65 characters for reading
  blocks.
- Labels: compact mono uppercase with letter spacing. In light theme they use
  accessible oxide ink `#A9381E`, not the brighter accent.

## 4. Components

- **Primitive controls:** Customize existing shadcn-svelte primitives so every
  current use receives the shared radius, color, focus and tactile states.
- **CustomButton:** A semantic wrapper over the shadcn Button for signature
  actions that need a trailing icon in a circular inset. Do not use it for
  ordinary icon buttons or every action.
- **Surface:** A custom double-bezel composition for product previews, major
  decisions and media. Outer shell: recess fill, low-opacity ink hairline,
  `1.8125rem` radius and `0.625rem` padding. Inner: surface fill and
  `1.25rem` radius. Do not nest ordinary cards inside it.
- **Buttons:** Pill-shaped. Primary uses Signal Oxide. Secondary uses Ink.
  Quiet uses Surface. Ghost is transparent. Press feedback is `scale(.98)` or
  a one-pixel transform; focus remains obvious.
- **Inputs:** `0.875rem` radius, label above, helper or error below, oxide
  focus ring. Placeholders never replace labels.
- **Dialogs and menus:** Surface fill, low-opacity ink hairline, soft
  canvas-tinted shadow, no black drop shadow.
- **Loading:** Skeletons match the eventual rows and panels. Never render a bare
  `Loading…` line where a panel will be: it collapses the layout and shoves
  content down when data lands. Use `custom/skeleton-rows.svelte` and pick
  `rows` / `height` to match what actually renders. Never hand-roll
  `animate-pulse`: the `Skeleton` primitive carries the `motion-safe:` guard.
- **Empty:** An empty state names the next action, never just the absence.
  "No backups yet." is a dead end; "No backups yet. Run one now, or wait for the
  next scheduled one." is not. Where there is room, compose `ui/empty` (icon,
  title, description, and the action itself as a button). Inside a dialog or a
  compact card, skip the composition and put the next action in the sentence.
  When several sections stack on one tab, only the first should carry a full
  empty state; repeating "nothing here" three times is noise, not guidance.
- **Errors:** Explain the next action in context, same as empty.

## 5. Layout Principles

- The desktop application shell is a restrained navigation island/sidebar with
  a stable page header and constrained content width.
- Use whitespace and sparse separators for routine groups. Reserve raised
  surfaces and double bezels for meaningful hierarchy.
- Use CSS Grid for responsive groups. At less than `768px`, collapse all
  multi-column content to one column and preserve 44px tap targets.
- Product screens may be dense, but never use generic three-equal-card grids
  as decoration.

## 6. Motion & Interaction

- Animate only `transform` and `opacity` with
  `cubic-bezier(0.32, 0.72, 0, 1)`.
- All nonessential animation is disabled under `prefers-reduced-motion`.
- Motion communicates press, selection, opening, closing, or navigation state.
  No perpetual decoration and no scroll listeners.

## 7. Banned Patterns

- Inter, generic gray palettes, pure black, neon or outer glows.
- Heavy black borders, dark drop shadows, gradients, and decorative bento
  grids.
- Generic three-column card layouts, stacked cards inside cards, or card
  elevation without hierarchy.
- Hand-drawn icons, emoji UI, custom cursors, artificial product screenshots,
  and motion that carries information alone.

## 8. Guided Onboarding

- Use a next-action checklist, never a spotlight tour or forced wizard.
- Both deployment and notification guides are available from the start; one is active at a time.
- The compact overlay defaults to the right edge and expands in place to show all steps.
- Alternate placement is the sidebar bottom. Mobile uses the same bottom overlay and grows upward.
- Automatic completion from resource state takes precedence over manual completion.
- Manual task completion and global dismissal never create or misrepresent infrastructure.
- The overlay never captures focus, opens automatically after dismissal, or blocks navigation.
