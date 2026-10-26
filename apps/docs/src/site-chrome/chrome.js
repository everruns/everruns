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

  // Newsletter forms (the footer's, and SubscribeForm.astro on /cloud). The
  // request goes out as JSON and the answer shows inline. The source is the
  // page the visitor is on, which for the docs is not the page the footer was
  // rendered on.
  //
  // A new address gets a welcome email, so the server wants a Cloudflare
  // Turnstile token (invisible widget). The script loads on first interaction
  // with a form, not on every page view.
  let turnstileScript;
  function loadTurnstile() {
    turnstileScript ??= new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src =
        'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
      script.async = true;
      script.onload = () => resolve(window.turnstile);
      script.onerror = reject;
      document.head.appendChild(script);
    });
    return turnstileScript;
  }

  function turnstileFor(form) {
    const sitekey = form.dataset.turnstileSitekey;
    const container = form.querySelector('[data-er-turnstile]');
    let widget;
    let token = '';
    let waiting = [];
    const settle = (value) => {
      token = value;
      waiting.forEach((resolve) => resolve(value));
      waiting = [];
    };
    const render = async () => {
      if (!sitekey || !container) return;
      const turnstile = await loadTurnstile();
      widget ??= turnstile.render(container, {
        sitekey,
        callback: settle,
        'expired-callback': () => (token = ''),
        'error-callback': () => settle(''),
      });
    };
    return {
      render,
      async token() {
        if (!sitekey) return '';
        await render().catch(() => {});
        if (token) return token;
        return new Promise((resolve) => {
          waiting.push(resolve);
          setTimeout(() => resolve(''), 10000);
        });
      },
      reset() {
        token = '';
        if (widget !== undefined) window.turnstile?.reset(widget);
      },
    };
  }

  document.querySelectorAll('form[data-er-subscribe]').forEach((form) => {
    const status = form.querySelector('[data-status]');
    const button = form.querySelector('button[type="submit"]');
    const challenge = turnstileFor(form);
    const warm = () => challenge.render().catch(() => {});
    form.addEventListener('focusin', warm, { once: true });
    form.addEventListener('pointerdown', warm, { once: true });
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
            turnstile: await challenge.token(),
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
        // A Turnstile token is single-use.
        challenge.reset();
        button.disabled = false;
      }
    });
  });
})();
