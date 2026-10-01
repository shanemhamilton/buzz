import type { QueryClient } from "@tanstack/react-query";

import { invalidateChannelMembersRosters } from "@/features/channels/rosterFreshness";
import { removeChannelMember } from "@/shared/api/tauri";
import type { Channel, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type AgentRosterCleanupAttempt = {
  agentPubkey: string;
  channelId: string;
};

export type AgentRosterCleanupResult = {
  failures: AgentRosterCleanupAttempt[];
  rosterRefreshError: string | null;
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Resolve every known membership for the supplied agents from both the relay
 * directory and the cached channel roster. The two sources overlap in the
 * normal case, but each can lag the other during deletion.
 */
export function agentRosterCleanupAttempts(
  agentPubkeys: readonly string[],
  channels: readonly Channel[],
  relayAgents: readonly RelayAgent[],
): AgentRosterCleanupAttempt[] {
  const channelIdsByPubkey = new Map<string, Set<string>>();
  const originalPubkeyByNormalized = new Map<string, string>();

  for (const pubkey of agentPubkeys) {
    const normalized = normalizePubkey(pubkey);
    originalPubkeyByNormalized.set(normalized, pubkey);
    channelIdsByPubkey.set(normalized, new Set());
  }

  for (const relayAgent of relayAgents) {
    const normalized = normalizePubkey(relayAgent.pubkey);
    const channelIds = channelIdsByPubkey.get(normalized);
    if (!channelIds) continue;
    for (const channelId of relayAgent.channelIds ?? []) {
      channelIds.add(channelId);
    }
  }

  for (const channel of channels) {
    for (const memberPubkey of channel.memberPubkeys) {
      const channelIds = channelIdsByPubkey.get(normalizePubkey(memberPubkey));
      if (channelIds) channelIds.add(channel.id);
    }
  }

  return [...channelIdsByPubkey].flatMap(([pubkey, channelIds]) => {
    const originalPubkey = originalPubkeyByNormalized.get(pubkey);
    if (!originalPubkey) return [];
    return [...channelIds].map((channelId) => ({
      agentPubkey: originalPubkey,
      channelId,
    }));
  });
}

/**
 * Remove already-deleted agents from every known roster and invalidate every
 * attempted roster read. A failed write is retained in the result so callers
 * can report partial deletion instead of claiming full cleanup.
 */
export async function cleanupAgentRosters({
  attempts,
  queryClient,
}: {
  attempts: readonly AgentRosterCleanupAttempt[];
  queryClient: Pick<QueryClient, "invalidateQueries">;
}): Promise<AgentRosterCleanupResult> {
  const settled = await Promise.allSettled(
    attempts.map(({ agentPubkey, channelId }) =>
      removeChannelMember(channelId, agentPubkey),
    ),
  );
  const failures = settled.flatMap((result, index) =>
    result.status === "rejected" ? [attempts[index]] : [],
  );

  try {
    await invalidateChannelMembersRosters(
      queryClient,
      attempts.map((attempt) => attempt.channelId),
    );
    return { failures, rosterRefreshError: null };
  } catch (error) {
    return { failures, rosterRefreshError: errorMessage(error) };
  }
}

export function agentRosterCleanupError({
  failures,
  rosterRefreshError,
}: AgentRosterCleanupResult) {
  const failedChannels = new Set(failures.map((failure) => failure.channelId));
  const failuresMessage =
    failedChannels.size > 0
      ? `could not remove the agent from ${failedChannels.size} channel${failedChannels.size === 1 ? "" : "s"}`
      : null;
  const refreshMessage = rosterRefreshError
    ? "could not refresh the affected channel rosters"
    : null;
  const details = [failuresMessage, refreshMessage].filter(
    (message): message is string => message !== null,
  );

  return details.length > 0
    ? `Agent deletion completed, but Buzz ${details.join(" and ")}.`
    : null;
}
