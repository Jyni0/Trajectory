import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Avatar, Button, Combobox, Dialog, Field, IconButton, Input, NavItem, Pill, Segmented, Switch } from "@/components";

/* The kit is Gravitation's design system; these pin its contract in Mail. */

describe("Button", () => {
  it("sizes follow the control scale: xs/sm → 24/28px rounded-lg, md → 36px rounded-xl", () => {
    render(
      <>
        <Button size="xs">xs</Button>
        <Button size="sm">sm</Button>
        <Button>md</Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "xs" })).toHaveClass("h-6", "rounded-lg");
    expect(screen.getByRole("button", { name: "sm" })).toHaveClass("h-7", "rounded-lg");
    expect(screen.getByRole("button", { name: "md" })).toHaveClass("h-9", "rounded-xl");
  });

  it("primary uses the theme accent; disabled blocks clicks", async () => {
    const onClick = vi.fn();
    const { rerender } = render(
      <Button variant="primary" onClick={onClick}>
        Send
      </Button>,
    );
    const btn = screen.getByRole("button", { name: "Send" });
    expect(btn).toHaveClass("bg-[var(--accent)]");
    await userEvent.click(btn);
    expect(onClick).toHaveBeenCalledTimes(1);
    rerender(
      <Button variant="primary" onClick={onClick} disabled>
        Send
      </Button>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("IconButton exposes its label as name and tooltip", () => {
    render(<IconButton label="Archive">x</IconButton>);
    expect(screen.getByRole("button", { name: "Archive" })).toHaveAttribute("title", "Archive");
  });
});

describe("Field + Input", () => {
  it("labels the control and accepts typing", async () => {
    function Controlled() {
      const [v, setV] = useState("");
      return (
        <Field label="Email">
          <Input value={v} onChange={(e) => setV(e.target.value)} />
        </Field>
      );
    }
    render(<Controlled />);
    const input = screen.getByLabelText("Email");
    await userEvent.type(input, "jane@example.com");
    expect(input).toHaveValue("jane@example.com");
  });

  it("md inputs are rounded-xl, sm inputs rounded-lg", () => {
    render(
      <>
        <Input aria-label="a" />
        <Input aria-label="b" size="sm" />
      </>,
    );
    expect(screen.getByLabelText("a")).toHaveClass("h-9", "rounded-xl");
    expect(screen.getByLabelText("b")).toHaveClass("h-7", "rounded-lg");
  });
});

describe("Switch", () => {
  function Harness({ disabled = false }: { disabled?: boolean }) {
    const [on, setOn] = useState(false);
    return <Switch on={on} onChange={setOn} ariaLabel="Notifications" disabled={disabled} />;
  }

  it("toggles on click and with the keyboard", async () => {
    render(<Harness />);
    const sw = screen.getByRole("switch", { name: "Notifications" });
    expect(sw).toHaveAttribute("aria-checked", "false");
    await userEvent.click(sw);
    expect(sw).toHaveAttribute("aria-checked", "true");
    expect(sw).toHaveClass("bg-[var(--accent)]");
    sw.focus();
    await userEvent.keyboard(" ");
    expect(sw).toHaveAttribute("aria-checked", "false");
  });

  it("ignores interaction when disabled", async () => {
    render(<Harness disabled />);
    await userEvent.click(screen.getByRole("switch"));
    expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false");
  });
});

describe("Segmented", () => {
  it("marks the chosen option and reports changes", async () => {
    const onChange = vi.fn();
    render(<Segmented value="all" onChange={onChange} options={[{ value: "all", label: "All" }, { value: "unread", label: "Unread" }]} />);
    expect(screen.getByRole("radio", { name: "All" })).toHaveAttribute("aria-checked", "true");
    await userEvent.click(screen.getByRole("radio", { name: "Unread" }));
    expect(onChange).toHaveBeenCalledWith("unread");
  });
});

describe("Dialog", () => {
  function Harness({ onClose = () => {} }: { onClose?: () => void }) {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button onClick={() => setOpen(true)}>Open</button>
        <Dialog
          open={open}
          label="Settings"
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        >
          <input aria-label="inside" data-autofocus />
          <button>Close it</button>
        </Dialog>
      </>
    );
  }

  it("renders a labelled rounded-3xl dialog and focuses [data-autofocus]", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    const dialog = await screen.findByRole("dialog", { name: "Settings" });
    expect(dialog).toHaveClass("rounded-3xl");
    expect(screen.getByLabelText("inside")).toHaveFocus();
  });

  it("closes on Escape and on the backdrop, not on clicks inside", async () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByLabelText("inside"));
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    await userEvent.click(await screen.findByTestId("modal-overlay"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("Combobox", () => {
  const OPTIONS = [
    { value: "tls", label: "SSL/TLS" },
    { value: "start_tls", label: "STARTTLS" },
    { value: "none", label: "None (insecure)" },
  ];
  function Harness({ searchable = true }: { searchable?: boolean }) {
    const [v, setV] = useState("tls");
    return <Combobox value={v} onChange={setV} options={OPTIONS} searchable={searchable} />;
  }

  it("opens a rounded-2xl listbox and selects with the mouse", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: /SSL\/TLS/ }));
    expect(screen.getByRole("listbox")).toHaveClass("rounded-2xl");
    await userEvent.click(screen.getByRole("option", { name: "STARTTLS" }));
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /STARTTLS/ })).toHaveAttribute("aria-expanded", "false");
  });

  it("filters while typing and picks with the keyboard", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: /SSL\/TLS/ }));
    await userEvent.type(screen.getByLabelText("Filter options"), "none");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    await userEvent.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: /None \(insecure\)/ })).toBeInTheDocument();
  });

  it("navigates with arrows when not searchable and closes on Escape", async () => {
    render(<Harness searchable={false} />);
    await userEvent.click(screen.getByRole("button", { name: /SSL\/TLS/ }));
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByRole("button", { name: /STARTTLS/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /STARTTLS/ }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });
});

describe("NavItem / Pill / Avatar", () => {
  it("NavItem highlights the active row", async () => {
    const onClick = vi.fn();
    render(
      <>
        <NavItem active onClick={onClick}>
          Inbox
        </NavItem>
        <NavItem>Sent</NavItem>
      </>,
    );
    expect(screen.getByRole("button", { name: "Inbox" })).toHaveClass("bg-[var(--hover-bg)]");
    expect(screen.getByRole("button", { name: "Sent" })).not.toHaveClass("bg-[var(--hover-bg)]");
    await userEvent.click(screen.getByRole("button", { name: "Inbox" }));
    expect(onClick).toHaveBeenCalled();
  });

  it("Avatar is round with initials; Pill renders its tone", () => {
    const { container } = render(
      <>
        <Avatar name="Jane Doe" email="jane@example.com" />
        <Pill tone="danger">Sync error</Pill>
      </>,
    );
    expect(container.querySelector("span.rounded-full")).toHaveTextContent("JD");
    expect(screen.getByText("Sync error")).toHaveClass("text-[var(--diff-del)]");
  });
});
