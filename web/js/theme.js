// Applies the saved light/dark theme before the page renders (kept out of index.html for the CSP).
try { const t = localStorage.getItem('so-theme'); if (t) document.documentElement.dataset.theme = t; } catch (e) { /* storage blocked */ }
