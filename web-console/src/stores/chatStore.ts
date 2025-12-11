/**
 * Chat state management store
 */

import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';
import type { TaskPlan } from '@/types';
import type { ExecutionResultEntry } from '@/utils/executionSummary';

export interface ExecutionSummaryStep {
  stepId: string;
  title: string;
  status: string;
  attempts: number;
  durationMs: number;
  error?: string | null;
}

export interface ExecutionSummary {
  success: boolean;
  stdout?: string;
  stderr?: string;
  artifactPath?: string;
  steps: ExecutionSummaryStep[];
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: Date;
  taskPlan?: TaskPlan;
  suggestions?: string[];
  executionSummary?: ExecutionSummary;
  executionResults?: ExecutionResultEntry[];
}

interface ChatState {
  // State
  messages: ChatMessage[];
  isTyping: boolean;
  currentPlan: TaskPlan | null;

  // Actions
  addMessage: (message: Omit<ChatMessage, 'id' | 'timestamp'>) => void;
  clearMessages: () => void;
  setTyping: (isTyping: boolean) => void;
  setCurrentPlan: (plan: TaskPlan | null) => void;
}

let messageIdCounter = 0;

export const useChatStore = create<ChatState>()(
  immer((set) => ({
    // Initial state
    messages: [],
    isTyping: false,
    currentPlan: null,

    // Actions
    addMessage: (message) =>
      set((state) => {
        const newMessage: ChatMessage = {
          ...message,
          id: `msg-${++messageIdCounter}`,
          timestamp: new Date(),
        };
        state.messages.push(newMessage);
      }),

    clearMessages: () =>
      set((state) => {
        state.messages = [];
        state.currentPlan = null;
      }),

    setTyping: (isTyping) =>
      set((state) => {
        state.isTyping = isTyping;
      }),

    setCurrentPlan: (plan) =>
      set((state) => {
        state.currentPlan = plan;
      }),
  }))
);

// Selectors
export const selectLastMessage = (state: ChatState) =>
  state.messages[state.messages.length - 1];

export const selectMessageCount = (state: ChatState) => state.messages.length;
