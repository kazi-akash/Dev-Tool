import { lazy, Suspense } from "react";
import { HashRouter, Route, Routes } from "react-router-dom";
import { AppLayout } from "@/components/layout/AppLayout";
import { ActionsPage } from "@/features/actions/ActionsPage";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { DiagnosisPage } from "@/features/diagnosis/DiagnosisPage";
import { ProcessesPage } from "@/features/processes/ProcessesPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { StoragePage } from "@/features/storage/StoragePage";
import { SettingsProvider } from "@/lib/settings";

// Charts pull in a large library; load them only when History is opened.
const HistoryPage = lazy(() => import("@/features/history/HistoryPage"));

export function App() {
  return (
    <SettingsProvider>
      <HashRouter>
        <Routes>
          <Route element={<AppLayout />}>
            <Route index element={<DashboardPage />} />
            <Route path="processes" element={<ProcessesPage />} />
            <Route path="storage" element={<StoragePage />} />
            <Route path="diagnosis" element={<DiagnosisPage />} />
            <Route path="actions" element={<ActionsPage />} />
            <Route
              path="history"
              element={
                <Suspense fallback={<p className="text-sm text-muted-foreground">Loading charts...</p>}>
                  <HistoryPage />
                </Suspense>
              }
            />
            <Route path="settings" element={<SettingsPage />} />
          </Route>
        </Routes>
      </HashRouter>
    </SettingsProvider>
  );
}
