for (const button of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
  const label = button.textContent;
  let reset: ReturnType<typeof setTimeout> | undefined;
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(button.dataset.copy ?? '');
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Press ⌘C';
    }
    clearTimeout(reset);
    reset = setTimeout(() => (button.textContent = label), 1600);
  });
}

// Terminal output streams in line by line.
for (const term of document.querySelectorAll<HTMLElement>('.term[data-reveal]')) {
  term.querySelectorAll<HTMLElement>('.line').forEach((line, i) => line.style.setProperty('--i', String(i)));
}

const targets = document.querySelectorAll<HTMLElement>('[data-reveal]');
const nodes = document.querySelectorAll<HTMLElement>('main .node');

if ('IntersectionObserver' in window) {
  // A node fills once the rail's head (60% down the screen) reaches it, and
  // empties again when you scroll back above it.
  const rail = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) entry.target.classList.toggle('in', entry.isIntersecting);
    },
    { rootMargin: '100% 0px -40% 0px' },
  );
  nodes.forEach((el) => rail.observe(el));

  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add('in');
        observer.unobserve(entry.target);
      }
    },
    { rootMargin: '0px 0px -10% 0px' },
  );
  targets.forEach((el) => observer.observe(el));
} else {
  targets.forEach((el) => el.classList.add('in'));
  nodes.forEach((el) => el.classList.add('in'));
}
