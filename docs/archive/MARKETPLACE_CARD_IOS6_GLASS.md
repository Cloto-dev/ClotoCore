# Marketplace Card — iOS 6 Glass Style (Archived)

> **Status**: Archived (2026-03-15)
> **Reason**: Replaced with modern Linear/Vercel-style design.
> **Future use**: UI customization / theme system / Easter egg.

## Screenshot

See: `docs/images/marketplace-ios6-glass.png` (if captured)

## CSS Implementation

```html
<!-- Container -->
<div className="
  relative
  bg-glass-strong
  backdrop-blur-sm
  border border-edge
  rounded-lg p-3
  flex flex-col gap-2
  overflow-hidden
  shadow-[inset_0_1px_1px_rgba(255,255,255,0.1),inset_0_-1px_2px_rgba(0,0,0,0.2),0_0_15px_rgba(100,140,255,0.06)]
">
  <!-- Layer 1: Surface reflection (top bright, bottom subtle) -->
  <div className="
    absolute inset-0
    bg-gradient-to-b from-white/[0.08] via-transparent to-white/[0.03]
    pointer-events-none rounded-lg
  " />

  <!-- Layer 2: LCD backlight glow (brand-tinted inner light) -->
  <div className="
    absolute inset-[1px] rounded-[7px]
    bg-gradient-to-br from-brand/[0.04] via-transparent to-brand/[0.02]
    pointer-events-none
  " />

  <!-- Content goes here -->
</div>
```

## Design Notes

- **Inner shadow (3-layer)**: Top white highlight + bottom dark shadow + outer blue glow
- **Gradient layers**: Simulates glass surface reflection and LCD backlight
- **backdrop-blur-sm**: Frosted glass effect on background content
- **Visual impression**: iOS 6 era skeuomorphic glass panel with LCD backlight
- **Accidentally discovered**: During marketplace UI development, the combination of
  inner shadows + dual gradient layers produced a convincing retro glass effect
