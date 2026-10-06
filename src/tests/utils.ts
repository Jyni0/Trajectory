import { setApi } from "@/lib/api";
import { createMockBackend, type MockBackend, type MockOptions } from "@/lib/mockBackend";
import { useMailStore } from "@/store/mailStore";
import { useUiStore } from "@/store/uiStore";
import { resetSenderLogos } from "@/components/mail/SenderAvatar";

/** Fresh mock backend + pristine stores for every test. */
export function setupBackend(opts: MockOptions = { withDemoAccount: true, seedCount: 12 }): MockBackend {
  const backend = createMockBackend({ latencyMs: 0, ...opts });
  setApi(backend);
  resetSenderLogos();
  useMailStore.setState(useMailStore.getInitialState(), true);
  useUiStore.setState({ ...useUiStore.getInitialState(), toasts: [], composer: null, settingsOpen: false }, true);
  return backend;
}
