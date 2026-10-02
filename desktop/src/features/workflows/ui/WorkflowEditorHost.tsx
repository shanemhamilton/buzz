import { useWorkflowQuery } from "@/features/workflows/hooks";
import { WorkflowDialog } from "@/features/workflows/ui/WorkflowDialog";
import { WorkflowUnavailableDialog } from "@/features/workflows/ui/WorkflowUnavailableDialog";
import type { Channel, Workflow } from "@/shared/api/types";
import {
  workflowIdentityKey,
  workflowMatchesReference,
} from "@/shared/api/workflowTypes";
import type { WorkflowEditorPane } from "./workflowEditorPane";

/** Create target for the shared workflow editor. */
export type WorkflowEditorCreateTarget = {
  initialChannelId?: string;
  mode: "create";
  pane: WorkflowEditorPane;
};

/** Existing-workflow target for the shared workflow editor. */
export type WorkflowEditorWorkflowTarget = {
  channelId: string | null;
  mode: "detail" | "duplicate" | "edit";
  ownerPubkey: string;
  pane: WorkflowEditorPane;
  workflowId: string;
};

/**
 * What the workflow editor is currently pointed at, independent of how it was
 * opened. The Workflows route derives this from the URL; the channel-anchored
 * overlay derives it from local state so the channel stays behind the modal.
 */
export type WorkflowEditorTarget =
  | WorkflowEditorCreateTarget
  | WorkflowEditorWorkflowTarget;

type WorkflowEditorHostProps = {
  channels: Channel[];
  editor: WorkflowEditorTarget | null;
  onClose: () => void;
  onDeleteWorkflow: (workflow: Workflow) => void;
  onDuplicateWorkflow: (workflow: Workflow) => void;
  onEditWorkflow: (workflow: Workflow) => void;
  onEditorPaneChange: (pane: WorkflowEditorPane) => void;
  onTriggerWorkflow: (workflowId: string) => void;
  /**
   * Workflow the opening surface already holds for this target. Supplying it
   * skips the loading dialog the detail query would otherwise show first.
   */
  workflowHint?: Workflow;
};

/**
 * Renders the shared workflow editor (or its non-disclosing loading /
 * unavailable stand-in) for a target. Every surface that can open the editor
 * mounts this so none of them fork the editor's lifecycle.
 */
export function WorkflowEditorHost({
  channels,
  editor,
  onClose,
  onDeleteWorkflow,
  onDuplicateWorkflow,
  onEditWorkflow,
  onEditorPaneChange,
  onTriggerWorkflow,
  workflowHint,
}: WorkflowEditorHostProps) {
  const editorWorkflowReference =
    editor && editor.mode !== "create"
      ? {
          channelId: editor.channelId,
          id: editor.workflowId,
          ownerPubkey: editor.ownerPubkey,
        }
      : null;
  const editorWorkflowQuery = useWorkflowQuery(editorWorkflowReference);
  const editorWorkflow =
    workflowHint &&
    editorWorkflowReference &&
    workflowMatchesReference(workflowHint, editorWorkflowReference)
      ? workflowHint
      : editorWorkflowQuery.data;

  if (!editor) return null;

  const dialogKey =
    editor.mode !== "create" && editorWorkflowReference
      ? `${editor.mode}:${workflowIdentityKey(editorWorkflowReference)}`
      : editor.mode;

  if (editor.mode !== "create" && editorWorkflow === undefined) {
    return (
      <WorkflowUnavailableDialog
        loading={editorWorkflowQuery.isLoading}
        onOpenChange={(open) => {
          if (!open) onClose();
        }}
        onRetry={() => void editorWorkflowQuery.refetch()}
        open
      />
    );
  }

  return (
    <WorkflowDialog
      channels={channels}
      initialChannelId={
        editor.mode === "create" ? editor.initialChannelId : undefined
      }
      key={dialogKey}
      mode={editor.mode === "detail" ? "edit" : editor.mode}
      onDeleteWorkflow={onDeleteWorkflow}
      onDuplicateWorkflow={onDuplicateWorkflow}
      onEditWorkflow={onEditWorkflow}
      onEditorPaneChange={onEditorPaneChange}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      onTriggerWorkflow={onTriggerWorkflow}
      open
      pane={editor.pane}
      workflow={editorWorkflow}
    />
  );
}
