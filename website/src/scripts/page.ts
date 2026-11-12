for (const button of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(button.dataset.copy ?? '');
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Press ⌘C';
    }
    setTimeout(() => (button.textContent = 'Copy'), 1600);
  });
}

// Terminal output streams in line by line.
for (const term of document.querySelectorAll<HTMLElement>('.term[data-reveal]')) {
  term.querySelectorAll<HTMLElement>('.line').forEach((line, i) => line.style.setProperty('--i', String(i)));
}

const targets = document.querySelectorAll<HTMLElement>('[data-reveal]');

if ('IntersectionObserver' in window) {
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
}
