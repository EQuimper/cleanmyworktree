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
