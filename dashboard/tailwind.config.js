/** @type {import('tailwindcss').Config} */
export default {
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
        }
      }
    },
  },
  plugins: [],
}