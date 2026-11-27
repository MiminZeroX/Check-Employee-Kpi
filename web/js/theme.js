// Theme: "auto" (default) follows the device's light/dark setting; "light"/"dark" are explicit choices.
// Kept out of index.html so the Content-Security-Policy can forbid inline scripts.
try {
  const t = localStorage.getItem('so-theme');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
} catch (e) { /* storage blocked: follow the device */ }
