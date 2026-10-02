import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import {
  parseWorkflowEditorPane,
  serializeWorkflowEditorPane,
} from "@/features/workflows/ui/workflowEditorPane";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyWorkflowsRouteScreen } from "./lazyWorkflowsRouteScreen";

export const Route = createFileRoute("/workflows/$workflowId")({
  component: WorkflowRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    channel: typeof search.channel === "string" ? search.channel : null,
    owner: typeof search.owner === "string" ? search.owner : "",
    pane: serializeWorkflowEditorPane(parseWorkflowEditorPane(search.pane)),
    view:
      search.view === "edit" || search.view === "duplicate"
        ? search.view
        : undefined,
  }),
});

function WorkflowRouteComponent() {
  usePreviewFeatureWarning("workflows");
  const navigate = Route.useNavigate();
  const location = useLocation();
  const { workflowId } = Route.useParams();
  const { channel, owner, pane, view } = Route.useSearch();
  const hasOrigin =
    (location.state as { workflowEditorHasOrigin?: unknown } | undefined)
      ?.workflowEditorHasOrigin === true;
  const editor: import("@/features/workflows/ui/WorkflowsScreen").WorkflowEditorRoute =
    {
      hasOrigin,
      channelId: channel,
      mode:
        view === "duplicate"
          ? "duplicate"
          : view === "edit"
            ? "edit"
            : "detail",
      pane: parseWorkflowEditorPane(pane),
      ownerPubkey: owner,
      workflowId,
    };

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <LazyWorkflowsRouteScreen
        editor={editor}
        onEditorPaneChange={(nextPane) => {
          void navigate({
            replace: true,
            resetScroll: false,
            search: {
              channel,
              owner,
              pane: serializeWorkflowEditorPane(nextPane),
              view,
            },
          });
        }}
      />
    </React.Suspense>
  );
}
