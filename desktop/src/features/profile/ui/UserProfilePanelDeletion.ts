import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  deleteManagedAgentWithRules,
  type ManagedAgentActionResult,
} from "@/features/agents/lib/managedAgentControlActions";
import {
  agentRosterCleanupAttempts,
  agentRosterCleanupError,
  cleanupAgentRosters,
  type AgentRosterCleanupAttempt,
  type AgentRosterCleanupResult,
} from "@/features/agents/lib/agentRosterCleanup";
import type {
  AgentPersona,
  Channel,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";

type DeleteManagedAgentRulesContext = Omit<
  Parameters<typeof deleteManagedAgentWithRules>[0],
  "agent"
>;

type DeleteProfileManagedAgentContext = DeleteManagedAgentRulesContext & {
  removeAgentFromAllChannels: (
    pubkey: string,
    attempts?: readonly AgentRosterCleanupAttempt[],
  ) => Promise<AgentRosterCleanupResult>;
};

type DeleteProfileManagedAgentsForPersonaContext =
  DeleteProfileManagedAgentContext & {
    managedAgents: readonly ManagedAgent[];
    selectedAgent?: ManagedAgent;
  };

type UseProfileAgentDeletionInput = {
  channels?: readonly Channel[];
  deleteManagedAgent: DeleteManagedAgentRulesContext["deleteManagedAgent"];
  managedAgent?: ManagedAgent;
  managedAgents?: readonly ManagedAgent[];
  getAvailability: DeleteManagedAgentRulesContext["getAvailability"];
  relayAgents?: readonly RelayAgent[];
};

type ProfileManagedAgentDeletionResult = ManagedAgentActionResult & {
  cleanupAttempts?: AgentRosterCleanupAttempt[];
  cleanupError?: string;
};

type DeleteManagedAgentRecordOptions = {
  /** The direct profile-agent dialog has already obtained this confirmation. */
  skipRemoteDeleteConfirm?: boolean;
};

export function useProfileAgentDeletion({
  channels,
  deleteManagedAgent,
  managedAgent,
  managedAgents,
  getAvailability,
  relayAgents,
}: UseProfileAgentDeletionInput) {
  const queryClient = useQueryClient();
  const pendingCleanupByPubkeyRef = React.useRef(
    new Map<string, AgentRosterCleanupAttempt[]>(),
  );
  const removeAgentFromAllChannels = React.useCallback(
    async (
      agentPubkey: string,
      attempts?: readonly AgentRosterCleanupAttempt[],
    ) => {
      const cleanupAttempts =
        attempts ??
        agentRosterCleanupAttempts(
          [agentPubkey],
          channels ?? [],
          relayAgents ?? [],
        );
      return cleanupAgentRosters({
        attempts: cleanupAttempts,
        queryClient,
      });
    },
    [channels, queryClient, relayAgents],
  );
  const cleanupDeletedAgentRosters = React.useCallback(
    async (
      agentPubkeys: readonly string[],
      attempts?: readonly AgentRosterCleanupAttempt[],
    ) => {
      const cleanupAttempts =
        attempts ??
        agentRosterCleanupAttempts(
          agentPubkeys,
          channels ?? [],
          relayAgents ?? [],
        );
      return {
        attempts: cleanupAttempts,
        cleanup: await cleanupAgentRosters({
          attempts: cleanupAttempts,
          queryClient,
        }),
      };
    },
    [channels, queryClient, relayAgents],
  );

  const deleteManagedAgentRecord = React.useCallback(
    async (
      agentToDelete: ManagedAgent,
      { skipRemoteDeleteConfirm = true }: DeleteManagedAgentRecordOptions = {},
    ): Promise<ProfileManagedAgentDeletionResult> => {
      const pendingCleanup = pendingCleanupByPubkeyRef.current.get(
        agentToDelete.pubkey.toLowerCase(),
      );
      if (pendingCleanup) {
        const cleanup = await removeAgentFromAllChannels(
          agentToDelete.pubkey,
          pendingCleanup,
        );
        const cleanupError = agentRosterCleanupError(cleanup);
        if (cleanupError) {
          pendingCleanupByPubkeyRef.current.set(
            agentToDelete.pubkey.toLowerCase(),
            cleanup.failures.length > 0 ? cleanup.failures : pendingCleanup,
          );
          return { cleanupError, cleanupAttempts: pendingCleanup };
        }
        pendingCleanupByPubkeyRef.current.delete(
          agentToDelete.pubkey.toLowerCase(),
        );
        return {};
      }

      const result = await deleteProfileManagedAgent(agentToDelete, {
        channels: channels ?? [],
        deleteManagedAgent,
        getAvailability,
        relayAgents: relayAgents ?? [],
        removeAgentFromAllChannels,
        skipRemoteDeleteConfirm,
      });
      if (result.cleanupError && result.cleanupAttempts) {
        pendingCleanupByPubkeyRef.current.set(
          agentToDelete.pubkey.toLowerCase(),
          result.cleanupAttempts,
        );
      }
      return result;
    },
    [
      channels,
      deleteManagedAgent,
      getAvailability,
      relayAgents,
      removeAgentFromAllChannels,
    ],
  );

  const deleteManagedAgentsForPersona = React.useCallback(
    async (
      persona: AgentPersona,
    ): Promise<ProfileManagedAgentDeletionResult> => {
      const agentsByPubkey = new Map<string, ManagedAgent>();
      for (const agent of managedAgents ?? []) {
        if (agent.personaId === persona.id) {
          agentsByPubkey.set(agent.pubkey, agent);
        }
      }
      if (managedAgent?.personaId === persona.id) {
        agentsByPubkey.set(managedAgent.pubkey, managedAgent);
      }

      for (const agent of agentsByPubkey.values()) {
        // Built-in persona removal has no AgentDeleteConfirmDialog. It must
        // retain the deployed-agent confirmation, including unknown presence.
        const result = await deleteManagedAgentRecord(agent, {
          skipRemoteDeleteConfirm: false,
        });
        if (result.cancelled || result.cleanupError) return result;
      }
      return {};
    },
    [deleteManagedAgentRecord, managedAgent, managedAgents],
  );

  return {
    deleteManagedAgentRecord,
    deleteManagedAgentsForPersona,
    cleanupDeletedAgentRosters,
    removeAgentFromAllChannels,
  };
}

export async function deleteProfileManagedAgent(
  agent: ManagedAgent,
  context: DeleteProfileManagedAgentContext,
): Promise<ProfileManagedAgentDeletionResult> {
  const {
    channels,
    relayAgents,
    removeAgentFromAllChannels,
    ...deleteContext
  } = context;
  const result = await deleteManagedAgentWithRules({
    agent,
    channels,
    relayAgents,
    ...deleteContext,
  });
  if (result.cancelled) return result;

  const attempts = agentRosterCleanupAttempts(
    [agent.pubkey],
    channels,
    relayAgents,
  );
  const cleanup = await removeAgentFromAllChannels(agent.pubkey, attempts);
  const cleanupError = agentRosterCleanupError(cleanup);
  return cleanupError
    ? {
        ...result,
        cleanupAttempts:
          cleanup.failures.length > 0 ? cleanup.failures : attempts,
        cleanupError,
      }
    : result;
}

export async function deleteProfileManagedAgentsForPersona(
  persona: AgentPersona,
  context: DeleteProfileManagedAgentsForPersonaContext,
): Promise<ProfileManagedAgentDeletionResult> {
  const { managedAgents, selectedAgent, ...deleteContext } = context;
  const agentsByPubkey = new Map<string, ManagedAgent>();

  for (const agent of managedAgents) {
    if (agent.personaId === persona.id) {
      agentsByPubkey.set(agent.pubkey, agent);
    }
  }

  if (selectedAgent?.personaId === persona.id) {
    agentsByPubkey.set(selectedAgent.pubkey, selectedAgent);
  }

  for (const agent of agentsByPubkey.values()) {
    const result = await deleteProfileManagedAgent(agent, deleteContext);
    if (result.cancelled || result.cleanupError) return result;
  }

  return {};
}
