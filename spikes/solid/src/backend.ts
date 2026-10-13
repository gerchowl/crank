// The one Backend instance for the app + a thin adapter for contract gaps (see README
// "Backend issues"). App code imports `backend` from here, never createMockBackend directly.
import { createMockBackend, type Backend, type MetricId, type MetricInfo } from "@neo/shared";

export const backend: Backend = createMockBackend({ speed: 10 });

const byId = new Map<MetricId, MetricInfo>(backend.metrics.map((m) => [m.id, m]));

export function metricInfo(id: MetricId): MetricInfo {
  const m = byId.get(id);
  if (!m) throw new Error(`unknown metric ${id}`);
  return m;
}

export const ftp = backend.athlete.ftp;
