import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Workflow } from "@/shared/api/types";
import { workflowIdentityKey } from "@/shared/api/workflowTypes";
import { getWorkflowTriggerConfig } from "./workflowDefinition";
import {
  type WorkflowAuthorPresentation,
  workflowTriggerAuthorPubkey,
} from "./useWorkflowAuthorPresentation";

export type WorkflowCardAuthorPresentation = Omit<
  WorkflowAuthorPresentation,
  "description"
>;

type WorkflowAuthorLookup = {
  pubkey: string;
  workflowKey: string;
};

export function workflowAuthorLookups(
  workflows: readonly Workflow[],
): WorkflowAuthorLookup[] {
  return workflows.flatMap((workflow) => {
    const trigger = getWorkflowTriggerConfig(workflow.definition);
    const pubkey = trigger ? workflowTriggerAuthorPubkey(trigger) : null;
    return pubkey
      ? [{ pubkey, workflowKey: workflowIdentityKey(workflow) }]
      : [];
  });
}

export function useWorkflowListAuthorPresentations(
  workflows: readonly Workflow[],
): Map<string, WorkflowCardAuthorPresentation> {
  const identityQuery = useIdentityQuery();
  const lookups = workflowAuthorLookups(workflows);
  const pubkeys = [...new Set(lookups.map(({ pubkey }) => pubkey))];
  const profilesQuery = useUsersBatchQuery(pubkeys);

  return new Map(
    lookups.map(({ pubkey, workflowKey }) => {
      const profile = profilesQuery.data?.profiles[pubkey];
      const loading = !profile && profilesQuery.isPending;
      return [
        workflowKey,
        {
          avatarUrl: profile?.avatarUrl ?? null,
          isAgent: profile?.isAgent === true,
          label: loading
            ? null
            : resolveUserLabel({
                currentPubkey: identityQuery.data?.pubkey,
                profiles: profilesQuery.data?.profiles,
                pubkey,
              }),
          loading,
          pubkey,
        },
      ];
    }),
  );
}
