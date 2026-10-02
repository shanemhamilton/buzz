import assert from "node:assert/strict";
import test from "node:test";

import { workflowAuthorLookups } from "./useWorkflowListAuthorPresentations.ts";

function workflow(
  index,
  pubkey,
  ownerPubkey = `${index}`.repeat(64).slice(0, 64),
) {
  return {
    id: `workflow-${index}`,
    channelId: "channel-1",
    ownerPubkey,
    definition: {
      trigger: {
        on: "message_posted",
        filter: `trigger_author == "${pubkey}"`,
      },
      steps: [],
    },
  };
}

test("keeps author presentations separate for duplicate UUIDs", () => {
  const sharedId = "workflow-shared";
  const first = {
    ...workflow(1, "a".repeat(64), "c".repeat(64)),
    id: sharedId,
  };
  const second = {
    ...workflow(2, "b".repeat(64), "d".repeat(64)),
    id: sharedId,
  };

  const lookups = workflowAuthorLookups([first, second]);
  assert.equal(lookups.length, 2);
  assert.notEqual(lookups[0].workflowKey, lookups[1].workflowKey);
});

test("collects configured authors for one list-level batch", () => {
  const authors = ["a".repeat(64), "b".repeat(64), "a".repeat(64)];
  const lookups = authors.flatMap((pubkey, index) =>
    workflowAuthorLookups([workflow(index, pubkey)]),
  );

  assert.equal(lookups.length, 3);
  assert.deepEqual(
    [...new Set(lookups.map(({ pubkey }) => pubkey))],
    authors.slice(0, 2),
  );
});
