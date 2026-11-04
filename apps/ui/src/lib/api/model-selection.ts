import { changeReasonOptions } from "./change-history";
import { api } from "./client";
import { recordModelSelection, type ModelSelectionReceipt } from "./write-observer";
export async function applyModelSelection(
  changes: { id: string; provider_id: string; expected_enabled: boolean; enabled: boolean }[],
) {
  const result = (await api.post<ModelSelectionReceipt[]>("/v1/models/selection", { changes }))
    .data;
  recordModelSelection(result);
  return result;
}
export async function undoModelSelection(
  changes: { id: string; from_revision: number; expected_revision: number }[],
) {
  return (
    await api.post<ModelSelectionReceipt[]>(
      "/v1/models/selection/undo",
      { changes },
      changeReasonOptions("Undo reviewed model selection"),
    )
  ).data;
}
