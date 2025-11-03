/**
 * Screenshot state management store
 */

import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { ScreenshotFrame } from '@/types';

interface ScreenshotState {
  // State
  frames: Map<string, ScreenshotFrame[]>; // taskId -> frames
  currentFrame: Map<string, ScreenshotFrame>; // taskId -> current frame
  subscriptions: Set<string>; // taskIds being streamed
  maxFramesPerTask: number;

  // Actions
  addFrame: (taskId: string, frame: ScreenshotFrame) => void;
  setCurrentFrame: (taskId: string, frame: ScreenshotFrame) => void;
  clearFrames: (taskId: string) => void;
  subscribe: (taskId: string) => void;
  unsubscribe: (taskId: string) => void;
}

export const useScreenshotStore = create<ScreenshotState>()(
  immer((set) => ({
    // Initial state
    frames: new Map(),
    currentFrame: new Map(),
    subscriptions: new Set(),
    maxFramesPerTask: 50,

    // Actions
    addFrame: (taskId, frame) =>
      set((state) => {
        let taskFrames = state.frames.get(taskId);
        if (!taskFrames) {
          taskFrames = [];
          state.frames.set(taskId, taskFrames);
        }

        taskFrames.push(frame);

        // Keep only last N frames
        if (taskFrames.length > state.maxFramesPerTask) {
          taskFrames.shift();
        }

        state.currentFrame.set(taskId, frame);
      }),

    setCurrentFrame: (taskId, frame) =>
      set((state) => {
        state.currentFrame.set(taskId, frame);
      }),

    clearFrames: (taskId) =>
      set((state) => {
        state.frames.delete(taskId);
        state.currentFrame.delete(taskId);
      }),

    subscribe: (taskId) =>
      set((state) => {
        state.subscriptions.add(taskId);
      }),

    unsubscribe: (taskId) =>
      set((state) => {
        state.subscriptions.delete(taskId);
      }),
  }))
);

// Selectors
export const selectCurrentFrame = (taskId: string) => (state: ScreenshotState) =>
  state.currentFrame.get(taskId);

export const selectFrameHistory = (taskId: string) => (state: ScreenshotState) =>
  state.frames.get(taskId) ?? [];

export const selectIsSubscribed = (taskId: string) => (state: ScreenshotState) =>
  state.subscriptions.has(taskId);
