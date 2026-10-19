// Behaviour for the shared site chrome (see chrome.css): the header gains a
// border once the page scrolls, the mobile menu toggles open, and the footer's
// newsletter form submits in place.
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

  // Without JavaScript the form posts natively; with it, the request goes out
  // as JSON and the answer shows inline. The source is the page the visitor
  // is on, which for the docs is not the page the footer was rendered on.
  document.querySelectorAll('form[data-er-subscribe]').forEach((form) => {
    const status = form.querySelector('[data-status]');
    const button = form.querySelector('button[type="submit"]');
    form.addEventListener('submit', async (event) => {
      event.preventDefault();
      if (!status || !button) return;
      const data = new FormData(form);
      status.textContent = '';
      status.classList.remove('is-error', 'is-success');
      button.disabled = true;
      try {
        const response = await fetch(form.action, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            Accept: 'application/json',
          },
          body: JSON.stringify({
            email: data.get('email'),
            website: data.get('website'),
            list: data.get('list'),
            source: window.location.pathname,
          }),
        });
        const result = await response.json();
        status.textContent = result.message;
        status.classList.add(result.ok ? 'is-success' : 'is-error');
        if (result.ok) {
          form.reset();
          // Conversion event; see src/components/GoogleTag.astro.
          window.gtag?.('event', 'subscribe', {
            list: String(data.get('list') ?? ''),
            source: window.location.pathname,
            page_path: window.location.pathname,
          });
        }
      } catch {
        status.textContent = 'Subscription failed. Please try again later.';
        status.classList.add('is-error');
      } finally {
        button.disabled = false;
      }
    });
  });
})();
