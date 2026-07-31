// @vitest-environment jsdom

import React, { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Modal } from './Modal';

vi.mock('@/infrastructure/i18n', () => ({
  useI18n: () => ({
    t: (key: string) => key,
  }),
}));

describe('Modal Escape handling', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  const pressEscape = () => {
    document.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
      cancelable: true,
    }));
  };

  it('closes a standalone modal', async () => {
    const onClose = vi.fn();

    await act(async () => {
      root.render(
        <Modal isOpen onClose={onClose} title="Standalone">
          Content
        </Modal>,
      );
    });

    act(pressEscape);

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes only the topmost modal for each Escape press', async () => {
    const parentClosed = vi.fn();
    const childClosed = vi.fn();

    const NestedModals = () => {
      const [parentOpen, setParentOpen] = useState(true);
      const [childOpen, setChildOpen] = useState(true);

      return (
        <>
          <Modal
            isOpen={parentOpen}
            onClose={() => {
              parentClosed();
              setParentOpen(false);
            }}
            title="Parent"
          >
            Parent content
          </Modal>
          <Modal
            isOpen={childOpen}
            onClose={() => {
              childClosed();
              setChildOpen(false);
            }}
            title="Child"
          >
            Child content
          </Modal>
        </>
      );
    };

    await act(async () => {
      root.render(<NestedModals />);
    });

    act(pressEscape);

    expect(childClosed).toHaveBeenCalledTimes(1);
    expect(parentClosed).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Parent content');
    expect(document.body.textContent).not.toContain('Child content');

    act(pressEscape);

    expect(childClosed).toHaveBeenCalledTimes(1);
    expect(parentClosed).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).not.toContain('Parent content');
  });

  it('removes a closed child from the stack before handling the next Escape', async () => {
    const parentClosed = vi.fn();

    const ReopenedChild = () => {
      const [childOpen, setChildOpen] = useState(true);

      return (
        <>
          <Modal isOpen onClose={parentClosed} title="Parent">
            Parent content
          </Modal>
          <Modal isOpen={childOpen} onClose={() => setChildOpen(false)} title="Child">
            Child content
          </Modal>
          {!childOpen ? <button onClick={() => setChildOpen(true)}>Reopen child</button> : null}
        </>
      );
    };

    await act(async () => {
      root.render(<ReopenedChild />);
    });

    act(pressEscape);
    const reopen = Array.from(document.body.querySelectorAll('button'))
      .find(button => button.textContent === 'Reopen child');
    expect(reopen).toBeTruthy();

    act(() => reopen?.click());
    act(pressEscape);

    expect(parentClosed).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Parent content');

    act(pressEscape);

    expect(parentClosed).toHaveBeenCalledTimes(1);
  });

});
