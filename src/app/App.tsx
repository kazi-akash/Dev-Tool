import { HashRouter, Route, Routes } from "react-router-dom";
import { AppLayout } from "@/components/layout/AppLayout";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { ProcessesPage } from "@/features/processes/ProcessesPage";
import { StoragePage } from "@/features/storage/StoragePage";
import { DiagnosisPage } from "@/features/diagnosis/DiagnosisPage";
import { ActionsPage } from "@/features/actions/ActionsPage";
import { HistoryPage } from "@/features/history/HistoryPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { SettingsProvider } from "@/lib/settings";

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
            <Route path="history" element={<HistoryPage />} />
            <Route path="settings" element={<SettingsPage />} />
          </Route>
        </Routes>
      </HashRouter>
    </SettingsProvider>
  );
}
