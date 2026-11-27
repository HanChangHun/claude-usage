// Opt-in local API for CLIs and agents (Settings → Local API). The backend
// only listens on 127.0.0.1 while this toggle is on; the choice is kept in
// localStorage like the other provider toggles.
const STORAGE_KEY = 'local-api-enabled';

export async function initLocalApi() {
  const toggle = document.getElementById('localApiToggle');
  const help = document.getElementById('localApiHelp');
  const idleText = help.textContent;
  let busy = false;

  function show(text, state = '') {
    help.textContent = text;
    help.dataset.state = state;
  }

  async function apply(enabled) {
    busy = true;
    toggle.disabled = true;
    try {
      const port = await window.__TAURI__.core.invoke('set_local_api', { enabled });
      toggle.checked = enabled;
      localStorage.setItem(STORAGE_KEY, String(enabled));
      show(enabled && port ? `Serving read-only usage at http://127.0.0.1:${port}/v1/usage` : idleText);
    } catch (error) {
      toggle.checked = false;
      localStorage.setItem(STORAGE_KEY, 'false');
      show(typeof error === 'string' ? error : 'Could not start the local API.', 'error');
    } finally {
      busy = false;
      toggle.disabled = false;
    }
  }

  toggle.addEventListener('change', () => {
    if (!busy) void apply(toggle.checked);
  });
  toggle.checked = localStorage.getItem(STORAGE_KEY) === 'true';
  if (toggle.checked) await apply(true);
}
