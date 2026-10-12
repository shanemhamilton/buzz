import type { AcpCommandCandidate } from "@/shared/api/acpCommands";
import { acpCommandPickerState } from "./acpCommandPicker";
import { PersonaDropdownField } from "./PersonaDropdownField";

type AcpCommandFieldProps = {
  candidates: AcpCommandCandidate[];
  id?: string;
  label?: string;
  description?: string;
  disabled: boolean;
  onValueChange: (value: string) => void;
  value: string;
};

export function AcpCommandField({
  candidates,
  id = "persona-acp-command",
  label = "ACP command",
  description = "Selects the ACP transport used when this agent is deployed.",
  disabled,
  onValueChange,
  value,
}: AcpCommandFieldProps) {
  const picker = acpCommandPickerState(value, candidates);
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium text-foreground" htmlFor={id}>
        {label}
      </label>
      <PersonaDropdownField
        disabled={disabled}
        id={id}
        onValueChange={onValueChange}
        options={picker.options}
        placeholder="Choose an ACP command"
        value={picker.selectValue}
      />
      <p className="text-xs text-muted-foreground">{description}</p>
    </div>
  );
}
