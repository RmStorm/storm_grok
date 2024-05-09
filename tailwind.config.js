/** @type {import('tailwindcss').Config} */
module.exports = {
  content: {
    files: ["*.html", "./app/src/**/*.rs"],
  },
  safelist: ["hidden"],
  theme: {
    extend: {},
  },
  plugins: [require("@tailwindcss/typography")],
};
