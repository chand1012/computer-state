export function Switch({ checked, onChange, disabled, label }: { checked: boolean; onChange: (checked: boolean) => void; disabled?: boolean; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} disabled={disabled} className={`switch ${checked ? "checked" : ""}`} onClick={() => onChange(!checked)}>
      <span />
    </button>
  );
}
