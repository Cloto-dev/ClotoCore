/** @type {import('tailwindcss').Config} */
// Every value here reads a token from src/index.css; the numbers themselves
// live there (docs/DESIGN_PHILOSOPHY.md §4).
export default {
  darkMode: 'class',
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    // Replaced, not extended: three radii and no others (§4.4).
    borderRadius: {
      none: '0',
      sm: 'var(--radius-tool)',
      DEFAULT: 'var(--radius-tool)',
      md: 'var(--radius-tool)',
      lg: 'var(--radius-surface)',
      xl: 'var(--radius-surface)',
      '2xl': 'var(--radius-surface)',
      '3xl': 'var(--radius-surface)',
      soft: 'var(--radius-soft)',
      full: '9999px',
    },
    extend: {
      fontFamily: {
        sans: ['var(--font-sans)'],
        mono: ['var(--font-mono)'],
      },
      fontSize: {
        xs: ['var(--text-note)', { lineHeight: '1.125rem' }],
        sm: ['var(--text-ui)', { lineHeight: '1.25rem' }],
      },
      colors: {
        // The present agent's colour — the only accent (§4.2).
        agent: {
          DEFAULT: 'hsl(var(--agent) / <alpha-value>)',
          ink: 'hsl(var(--agent-ink) / <alpha-value>)',
        },
        surface: {
          base: 'hsl(var(--surface-base) / <alpha-value>)',
          primary: 'hsl(var(--surface-primary) / <alpha-value>)',
          secondary: 'hsl(var(--surface-secondary) / <alpha-value>)',
          panel: 'hsl(var(--surface-panel) / <alpha-value>)',
          field: 'hsl(var(--surface-field) / <alpha-value>)',
          control: 'hsl(var(--surface-control) / <alpha-value>)',
        },
        content: {
          primary: 'hsl(var(--text-primary) / <alpha-value>)',
          secondary: 'hsl(var(--text-secondary) / <alpha-value>)',
          tertiary: 'hsl(var(--text-tertiary) / <alpha-value>)',
          muted: 'hsl(var(--text-muted) / <alpha-value>)',
        },
        edge: {
          DEFAULT: 'hsl(var(--border-default) / <alpha-value>)',
          subtle: 'hsl(var(--border-subtle) / <alpha-value>)',
        },
        mgp: {
          DEFAULT: 'rgb(var(--mgp-primary) / <alpha-value>)',
          accent: 'rgb(var(--mgp-accent) / <alpha-value>)',
          'accent-light': 'rgb(var(--mgp-accent-light) / <alpha-value>)',
          surface: 'rgb(var(--mgp-surface) / <alpha-value>)',
        },
      },
    },
  },
  plugins: [],
}
