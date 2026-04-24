/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{ts,tsx}', '../src/http/**/*.rs'],
  theme: {
    extend: {
      colors: {
        sand: {
          50: '#fbf8f3',
          100: '#f3ede2',
          200: '#e7dcc8',
          300: '#d7c4a4'
        },
        stoneink: {
          700: '#48504a',
          800: '#28302c',
          900: '#161d1a'
        },
        tealbrand: {
          500: '#0f766e',
          600: '#0b5e58',
          700: '#0a4d48'
        }
      },
      boxShadow: {
        panel: '0 18px 45px rgba(22, 29, 26, 0.08)',
        float: '0 14px 34px rgba(15, 118, 110, 0.10)'
      }
    }
  },
  plugins: []
};
