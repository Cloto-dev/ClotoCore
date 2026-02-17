/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        karin: {
          core: '#2e4de6',
          pink: '#ff007a',
          blue: '#00f2ff',
        },
        brand: {
          DEFAULT: 'rgb(var(--brand-primary) / <alpha-value>)',
        },
        surface: {
          base: 'rgb(var(--surface-base) / <alpha-value>)',
          primary: 'rgb(var(--surface-primary) / <alpha-value>)',
          secondary: 'rgb(var(--surface-secondary) / <alpha-value>)',
        },
        content: {
          primary: 'rgb(var(--text-primary) / <alpha-value>)',
          secondary: 'rgb(var(--text-secondary) / <alpha-value>)',
          tertiary: 'rgb(var(--text-tertiary) / <alpha-value>)',
          muted: 'rgb(var(--text-muted) / <alpha-value>)',
        },
        edge: {
          DEFAULT: 'rgb(var(--border-default) / <alpha-value>)',
          subtle: 'rgb(var(--border-subtle) / <alpha-value>)',
        },
      },
      backgroundColor: {
        glass: 'var(--surface-glass)',
        'glass-strong': 'var(--surface-glass-strong)',
        'glass-subtle': 'var(--surface-glass-subtle)',
      },
    },
  },
  plugins: [],
}
