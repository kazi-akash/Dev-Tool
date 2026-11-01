import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { useTrayNavigation } from "@/hooks/useTrayNavigation";
import { EmergencyBanner } from "@/components/EmergencyBanner";

export function AppLayout() {
  useTrayNavigation();
  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="min-w-0 flex-1 overflow-y-auto p-6">
        <EmergencyBanner />
        <Outlet />
      </main>
    </div>
  );
}
