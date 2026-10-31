import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { useTrayNavigation } from "@/hooks/useTrayNavigation";

export function AppLayout() {
  useTrayNavigation();
  return (
    <div className="flex h-full">
      <Sidebar />
      <main className="min-w-0 flex-1 overflow-y-auto p-6">
        <Outlet />
      </main>
    </div>
  );
}
