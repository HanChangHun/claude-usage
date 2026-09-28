export function renderAccount(account, provider) {
  if (!account) return '';
  const email = typeof account.email === 'string' ? account.email.trim() : '';
  const rawPlan = account.plan ?? account.planType;
  const plan = typeof rawPlan === 'string'
    ? rawPlan.trim().replace(/^claude_/, '').replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase())
    : '';
  if (!email && !plan) return '';
  const line = document.createElement('div');
  line.className = 'account-info';
  const label = document.createElement('span');
  label.className = 'account-provider';
  label.textContent = plan ? `${provider} · ${plan}` : provider;
  line.append(label);
  if (email) {
    const address = document.createElement('span');
    address.className = 'account-email';
    address.textContent = email;
    address.title = email;
    line.append(address);
  }
  return line.outerHTML;
}
