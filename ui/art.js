// Hand-drawn device illustrations (own artwork, no vendor assets).
window.ART = {
  mouse: `<svg width="180" height="320" viewBox="0 0 180 320">
    <defs><linearGradient id="mg" x1="0" x2="1"><stop offset="0" stop-color="#0c0c0e"/><stop offset=".5" stop-color="#26272b"/><stop offset="1" stop-color="#0c0c0e"/></linearGradient></defs>
    <path d="M90 8 C40 8 18 40 16 90 L14 200 C14 270 46 312 90 312 C134 312 166 270 166 200 L164 90 C162 40 140 8 90 8Z" fill="url(#mg)" stroke="#3a3b40"/>
    <path d="M16 140 C40 152 140 152 164 140" stroke="#3a3b40" fill="none"/>
    <path d="M90 8 L90 145" stroke="#3a3b40"/>
    <rect x="80" y="30" width="20" height="58" rx="8" fill="#050505" stroke="#444"/>
    <g stroke="#555">${[38,46,54,62,70,78].map(y=>`<line x1="84" x2="96" y1="${y}" y2="${y}"/>`).join('')}</g>
    <rect x="86" y="18" width="8" height="3" fill="#44d62c"/>
    <path d="M14 92 L8 96 L8 128 L14 132" fill="#1a1a1c" stroke="#333"/>
  </svg>`,
  headset: `<svg width="220" height="300" viewBox="0 0 220 300" fill="none" stroke="#6a6a6a" stroke-width="2">
    <rect x="96" y="6" width="40" height="80" rx="4"/>
    <path d="M100 86 L132 86 L132 104 L100 104Z"/>
    <path d="M70 120 C70 104 162 104 162 120"/>
    <rect x="62" y="116" width="108" height="122" rx="40" fill="#151515"/>
    <ellipse cx="116" cy="176" rx="38" ry="44" fill="#0e0e0e"/>
    <circle cx="116" cy="218" r="8"/>
    <path d="M70 216 C40 240 30 252 18 262" stroke-width="5"/>
    <circle cx="14" cy="266" r="10"/>
  </svg>`,
  dongle: `<svg width="90" height="240" viewBox="0 0 90 240" fill="none" stroke="#777" stroke-width="2"><path d="M40 0v40M50 0v40M38 40h14l8 60H30z"/><rect x="18" y="100" width="54" height="130" rx="4"/><circle cx="45" cy="210" r="6" fill="#ccc"/></svg>`,
  mouseIcon: `<svg width="22" height="26" viewBox="0 0 22 26" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="2" width="16" height="22" rx="8"/><path d="M11 6v5"/></svg>`,
  headsetIcon: `<svg width="26" height="24" viewBox="0 0 26 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 15v-3a10 10 0 0 1 20 0v3"/><rect x="2" y="14" width="6" height="8" rx="2"/><rect x="18" y="14" width="6" height="8" rx="2"/></svg>`,
  kbdIcon: `<svg width="26" height="20" viewBox="0 0 26 20" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="1" y="3" width="24" height="14" rx="2"/><path d="M5 8h2M10 8h2M15 8h2M20 8h1M6 13h14"/></svg>`,
  homeIcon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="#000"><circle cx="7" cy="16" r="3"/><circle cx="17" cy="16" r="3"/><circle cx="12" cy="6" r="3"/><path d="M7 16L12 6l5 10z" stroke="#000" stroke-width="2.5" fill="none"/></svg>`,
  sliders: `<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M6 3v18M12 3v18M18 3v18"/><rect x="4" y="13" width="4" height="3" fill="#151515"/><rect x="10" y="7" width="4" height="3" fill="#151515"/><rect x="16" y="12" width="4" height="3" fill="#151515"/></svg>`,
  pad: `<svg width="28" height="28" viewBox="0 0 24 24" fill="currentColor"><path d="M7 7h10a5 5 0 0 1 5 5v1a4 4 0 0 1-7 2.6l-.5-.6h-5l-.5.6A4 4 0 0 1 2 13v-1a5 5 0 0 1 5-5zm0 3v1.5H5.5v1H7V14h1v-1.5h1.5v-1H8V10zm9 .5a.9.9 0 1 0 0 1.8.9.9 0 0 0 0-1.8zm2 2a.9.9 0 1 0 0 1.8.9.9 0 0 0 0-1.8z"/></svg>`,
  film: `<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="4" y="3" width="16" height="18"/><path d="M8 3v18M16 3v18M4 7h4M4 11h4M4 15h4M16 7h4M16 11h4M16 15h4"/></svg>`,
  note: `<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M9 18V5l11-2v13"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/></svg>`,
  swords: `<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M4 4l11 11M20 4L9 15M13 17l4 4M11 17l-4 4M17 13l4 4M7 13l-4 4"/></svg>`,
  headph: `<svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="14" width="5" height="7" rx="2"/><rect x="16" y="14" width="5" height="7" rx="2"/></svg>`,
  mega: `<svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor"><path d="M3 10v4h4l6 4V6L7 10zM16 8l3-2M16 12h4M16 16l3 2" stroke="currentColor" stroke-width="1.6"/></svg>`,
  book: `<svg width="24" height="22" viewBox="0 0 24 22" fill="currentColor"><path d="M1 3c3-1.5 7-1.5 10 1v16c-3-2.5-7-2.5-10-1zM23 3c-3-1.5-7-1.5-10 1v16c3-2.5 7-2.5 10-1z"/></svg>`,
  layers: `<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"><circle cx="12" cy="12" r="10.5"/><path d="M6 10l6-3 6 3-6 3zM6 13l6 3 6-3M6 16l6 3 6-3"/></svg>`,
};
