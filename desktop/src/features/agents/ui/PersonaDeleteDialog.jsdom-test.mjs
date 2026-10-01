import assert from "node:assert/strict";
import test from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { PersonaDeleteDialog } from "./PersonaDeleteDialog.tsx";

const persona = {
  id: "scout",
  displayName: "Scout",
};

test("failed persona delete stays open with an actionable error", async () => {
  let rejectDelete;
  const deleting = new Promise((_, reject) => {
    rejectDelete = reject;
  });
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(
        React.createElement(PersonaDeleteDialog, {
          open: true,
          persona,
          onConfirm: () => deleting,
          onOpenChange: () => {
            throw new Error("the dialog must not close before deletion succeeds");
          },
        }),
      );
    });

    const deleteButton = [...document.body.querySelectorAll("button")].find(
      (button) => button.textContent === "Delete",
    );
    assert.ok(deleteButton, "delete control renders");

    await act(async () => {
      deleteButton.click();
      await Promise.resolve();
    });
    assert.equal(deleteButton.textContent, "Deleting…");
    assert.ok(
      document.body.querySelector('[role="alertdialog"]'),
      "dialog remains open while deletion is pending",
    );

    await act(async () => {
      rejectDelete(new Error("failed to secure deletion retry record"));
      await Promise.resolve();
    });
    const error = document.body.querySelector('[role="alert"]');
    assert.match(error?.textContent ?? "", /failed to secure deletion retry record/);
    assert.ok(
      document.body.querySelector('[role="alertdialog"]'),
      "failure keeps the confirmation open for retry",
    );
    assert.equal(deleteButton.textContent, "Delete");
  } finally {
    await act(async () => {
      root.unmount();
    });
    container.remove();
  }
});
