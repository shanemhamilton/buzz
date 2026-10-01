import * as React from "react";

import type { AgentPersona } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type PersonaDeleteDialogProps = {
  open: boolean;
  persona: AgentPersona | null;
  /** Number of managed-agent instances backed by this persona. Omit or pass 0 to suppress the instance-count sentence. */
  instanceCount?: number;
  onConfirm: (persona: AgentPersona) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
};

/**
 * Confirmation copy for deleting a persona. Pure so the cascade archival
 * disclosure stays unit-testable without a renderer: whenever instances are
 * cascade-deleted, each one's identity is also archived on the relay
 * (NIP-IA), and that durable side effect must be disclosed before the
 * destructive confirm — matching the direct agent-delete dialog.
 */
export function personaDeleteDescription(
  persona: AgentPersona | null,
  instanceCount: number,
): string {
  if (!persona) {
    return "Delete this agent.";
  }
  if (instanceCount === 0) {
    return `Delete ${persona.displayName}.`;
  }
  const cascade =
    instanceCount === 1
      ? "Also deletes 1 agent instance and archives its identity on the relay."
      : `Also deletes ${instanceCount} agent instances and archives their identities on the relay.`;
  return `Delete ${persona.displayName}. ${cascade}`;
}

export function PersonaDeleteDialog({
  open,
  persona,
  instanceCount = 0,
  onConfirm,
  onOpenChange,
}: PersonaDeleteDialogProps) {
  const [isConfirming, setIsConfirming] = React.useState(false);
  const [confirmError, setConfirmError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open || !persona?.id) return;
    setIsConfirming(false);
    setConfirmError(null);
  }, [open, persona?.id]);

  async function handleConfirm(event: React.MouseEvent<HTMLButtonElement>) {
    // AlertDialogAction closes by default. Keep this dialog mounted until the
    // cascade and roster cleanup complete so failures remain actionable.
    event.preventDefault();
    if (!persona || isConfirming) return;

    setConfirmError(null);
    setIsConfirming(true);
    try {
      await onConfirm(persona);
    } catch (error) {
      setConfirmError(
        error instanceof Error ? error.message : "Failed to delete agent.",
      );
    } finally {
      setIsConfirming(false);
    }
  }

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isConfirming) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete agent?</AlertDialogTitle>
          <AlertDialogDescription>
            {personaDeleteDescription(persona, instanceCount)}
          </AlertDialogDescription>
          {confirmError ? (
            <p className="text-sm text-destructive" role="alert">
              {confirmError}
            </p>
          ) : null}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isConfirming} type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              disabled={!persona || isConfirming}
              onClick={handleConfirm}
              type="button"
              variant="destructive"
            >
              {isConfirming ? "Deleting…" : "Delete"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
