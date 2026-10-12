// Behaviour for the shared site chrome (see chrome.css): the header gains a
// border once the page scrolls, and the mobile menu toggles open.
(() => {
  const header = document.querySelector('.er-header');
  if (header) {
    window.addEventListener('scroll', () => {
      if (window.scrollY > 10) {
        header.classList.add('er-h-scrolled');
      } else {
        header.classList.remove('er-h-scrolled');
      }
    });
  }

  const toggle = document.querySelector('.er-h-menu-toggle');
  const nav = document.querySelector('.er-h-nav');
  if (toggle && nav) {
    toggle.addEventListener('click', () => {
      const expanded = toggle.getAttribute('aria-expanded') === 'true';
      toggle.setAttribute('aria-expanded', String(!expanded));
      nav.classList.toggle('er-h-open');
    });
  }
})();
