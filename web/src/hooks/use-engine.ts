import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type {
  EngineConfigResponse,
  ModelInfo,
  ProviderInfo,
  ProviderModelsResponse,
} from '@/types/engine'
import type { FallbackHistoryResponse, RoutingConfig, RoutingStats } from '@/types/routing'

// ── Static provider catalog ──────────────────────────────────
// Mirrors the oxicode-ai provider registry so the frontend can
// display provider/model information without a dedicated API.

/** Known providers with display metadata. */
const PROVIDER_CATALOG: ProviderInfo[] = [
  {
    id: 'anthropic',
    name: 'Anthropic',
    category: 'major',
    hasKey: false,
    modelCount: 0,
    description: 'Claude models with extended thinking',
    envKey: 'ANTHROPIC_API_KEY',
  },
  {
    id: 'openai',
    name: 'OpenAI',
    category: 'major',
    hasKey: false,
    modelCount: 0,
    description: 'GPT, o-series, and Codex models',
    envKey: 'OPENAI_API_KEY',
  },
  {
    id: 'google',
    name: 'Google Gemini',
    category: 'major',
    hasKey: false,
    modelCount: 0,
    description: 'Gemini models with thinking and tool use',
    envKey: 'GOOGLE_API_KEY',
  },
  {
    id: 'groq',
    name: 'Groq',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Fast Llama, Mixtral, and Gemma inference',
    envKey: 'GROQ_API_KEY',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Unified gateway to 200+ models',
    envKey: 'OPENROUTER_API_KEY',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'DeepSeek-V3 and DeepSeek-R1',
    envKey: 'DEEPSEEK_API_KEY',
  },
  {
    id: 'mistral',
    name: 'Mistral',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Mistral and Codestral models',
    envKey: 'MISTRAL_API_KEY',
  },
  {
    id: 'xai',
    name: 'xAI (Grok)',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Grok models from xAI',
    envKey: 'XAI_API_KEY',
  },
  {
    id: 'cerebras',
    name: 'Cerebras',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Ultra-fast open model inference',
    envKey: 'CEREBRAS_API_KEY',
  },
  {
    id: 'fireworks',
    name: 'Fireworks',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Fast open-source model serving',
    envKey: 'FIREWORKS_API_KEY',
  },
  {
    id: 'github-copilot',
    name: 'GitHub Copilot',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'GitHub Copilot models (GPT-4, Claude)',
    envKey: 'GITHUB_COPILOT_TOKEN',
  },
  {
    id: 'huggingface',
    name: 'Hugging Face',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'Open model inference hub',
    envKey: 'HUGGINGFACE_API_KEY',
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    category: 'regional',
    hasKey: false,
    modelCount: 0,
    description: 'MiniMax-M2.7, abab models',
    envKey: 'MINIMAX_API_KEY',
  },
  {
    id: 'moonshotai',
    name: 'Moonshot AI (Kimi)',
    category: 'regional',
    hasKey: false,
    modelCount: 0,
    description: 'Kimi models from Moonshot AI',
    envKey: 'MOONSHOT_API_KEY',
  },
  {
    id: 'kimi-coding',
    name: 'Kimi Coding',
    category: 'regional',
    hasKey: false,
    modelCount: 0,
    description: 'Kimi Coding Plan — optimized for coding',
    envKey: 'KIMI_CODING_API_KEY',
  },
  {
    id: 'zai',
    name: 'Z.AI (GLM)',
    category: 'regional',
    hasKey: false,
    modelCount: 0,
    description: 'Z.AI GLM models (coding plan)',
    envKey: 'ZAI_API_KEY',
  },
  {
    id: 'opencode',
    name: 'OpenCode',
    category: 'open',
    hasKey: false,
    modelCount: 0,
    description: 'OpenCode coding agent gateway',
    envKey: '',
  },
]

/** Static model catalog (key providers with popular models). */
const MODEL_CATALOG: Record<string, ModelInfo[]> = {
  anthropic: [
    {
      id: 'claude-sonnet-4-20250514',
      name: 'Claude Sonnet 4',
      api: 'anthropic-messages',
      provider: 'anthropic',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 3,
      costOutput: 15,
      costCacheRead: 0.3,
      costCacheWrite: 3.75,
      contextWindow: 200000,
      maxTokens: 16384,
    },
    {
      id: 'claude-opus-4-20250514',
      name: 'Claude Opus 4',
      api: 'anthropic-messages',
      provider: 'anthropic',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 15,
      costOutput: 75,
      costCacheRead: 1.5,
      costCacheWrite: 18.75,
      contextWindow: 200000,
      maxTokens: 32768,
    },
    {
      id: 'claude-3-5-haiku-20241022',
      name: 'Claude 3.5 Haiku',
      api: 'anthropic-messages',
      provider: 'anthropic',
      reasoning: false,
      input: ['text', 'image'],
      costInput: 1,
      costOutput: 5,
      costCacheRead: 0.1,
      costCacheWrite: 1.25,
      contextWindow: 200000,
      maxTokens: 8192,
    },
  ],
  openai: [
    {
      id: 'gpt-4o',
      name: 'GPT-4o',
      api: 'openai-completions',
      provider: 'openai',
      reasoning: false,
      input: ['text', 'image'],
      costInput: 2.5,
      costOutput: 10,
      costCacheRead: 1.25,
      costCacheWrite: 0,
      contextWindow: 128000,
      maxTokens: 16384,
    },
    {
      id: 'gpt-4.1',
      name: 'GPT-4.1',
      api: 'openai-responses',
      provider: 'openai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 2,
      costOutput: 8,
      costCacheRead: 0.5,
      costCacheWrite: 0,
      contextWindow: 1047576,
      maxTokens: 32768,
    },
    {
      id: 'o3',
      name: 'o3',
      api: 'openai-responses',
      provider: 'openai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 2,
      costOutput: 8,
      costCacheRead: 0.5,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 100000,
    },
    {
      id: 'o4-mini',
      name: 'o4-mini',
      api: 'openai-responses',
      provider: 'openai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 1.1,
      costOutput: 4.4,
      costCacheRead: 0.275,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 100000,
    },
  ],
  google: [
    {
      id: 'gemini-2.5-pro',
      name: 'Gemini 2.5 Pro',
      api: 'google-generative-ai',
      provider: 'google',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 1.25,
      costOutput: 10,
      costCacheRead: 0.315,
      costCacheWrite: 0,
      contextWindow: 1048576,
      maxTokens: 65536,
    },
    {
      id: 'gemini-2.5-flash',
      name: 'Gemini 2.5 Flash',
      api: 'google-generative-ai',
      provider: 'google',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0.15,
      costOutput: 3.5,
      costCacheRead: 0.0375,
      costCacheWrite: 0,
      contextWindow: 1048576,
      maxTokens: 65536,
    },
  ],
  groq: [
    {
      id: 'llama-3.3-70b-versatile',
      name: 'Llama 3.3 70B',
      api: 'openai-completions',
      provider: 'groq',
      reasoning: false,
      input: ['text'],
      costInput: 0.59,
      costOutput: 0.79,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 32768,
    },
  ],
  openrouter: [
    {
      id: 'auto',
      name: 'Auto (Router)',
      api: 'openai-completions',
      provider: 'openrouter',
      reasoning: false,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 128000,
      maxTokens: 4096,
    },
  ],
  deepseek: [
    {
      id: 'deepseek-r1',
      name: 'DeepSeek R1',
      api: 'openai-completions',
      provider: 'deepseek',
      reasoning: true,
      input: ['text'],
      costInput: 0.55,
      costOutput: 2.19,
      costCacheRead: 0.14,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 16384,
    },
    {
      id: 'deepseek-chat',
      name: 'DeepSeek V3',
      api: 'openai-completions',
      provider: 'deepseek',
      reasoning: false,
      input: ['text'],
      costInput: 0.27,
      costOutput: 1.1,
      costCacheRead: 0.07,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 8192,
    },
  ],
  mistral: [
    {
      id: 'mistral-large-latest',
      name: 'Mistral Large',
      api: 'mistral-conversations',
      provider: 'mistral',
      reasoning: false,
      input: ['text'],
      costInput: 2,
      costOutput: 6,
      costCacheRead: 0.5,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 8192,
    },
  ],
  xai: [
    {
      id: 'grok-3',
      name: 'Grok 3',
      api: 'openai-completions',
      provider: 'xai',
      reasoning: false,
      input: ['text', 'image'],
      costInput: 3,
      costOutput: 15,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 16384,
    },
  ],
  cerebras: [
    {
      id: 'llama-3.3-70b',
      name: 'Llama 3.3 70B',
      api: 'openai-completions',
      provider: 'cerebras',
      reasoning: false,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 8192,
    },
  ],
  fireworks: [
    {
      id: 'llama-v3p3-70b-instruct',
      name: 'Llama 3.3 70B',
      api: 'openai-completions',
      provider: 'fireworks',
      reasoning: false,
      input: ['text'],
      costInput: 0.9,
      costOutput: 0.9,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 16384,
    },
  ],
  'github-copilot': [
    {
      id: 'gpt-4o',
      name: 'GPT-4o (Copilot)',
      api: 'openai-completions',
      provider: 'github-copilot',
      reasoning: false,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 128000,
      maxTokens: 16384,
    },
  ],
  huggingface: [
    {
      id: 'meta-llama/Llama-3.3-70B-Instruct',
      name: 'Llama 3.3 70B',
      api: 'openai-completions',
      provider: 'huggingface',
      reasoning: false,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 8192,
    },
  ],
  together: [
    {
      id: 'meta-llama/Llama-3.3-70B-Instruct-Turbo',
      name: 'Llama 3.3 70B Turbo',
      api: 'openai-completions',
      provider: 'together',
      reasoning: false,
      input: ['text'],
      costInput: 0.88,
      costOutput: 0.88,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 131072,
    },
    {
      id: 'Qwen/Qwen3-235B-A22B-Instruct-2507-tput',
      name: 'Qwen3 235B A22B',
      api: 'openai-completions',
      provider: 'together',
      reasoning: true,
      input: ['text'],
      costInput: 0.2,
      costOutput: 0.6,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 262144,
    },
    {
      id: 'Qwen/Qwen3-Coder-480B-A35B-Instruct-FP8',
      name: 'Qwen3 Coder 480B',
      api: 'openai-completions',
      provider: 'together',
      reasoning: false,
      input: ['text'],
      costInput: 2,
      costOutput: 2,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 262144,
    },
    {
      id: 'deepseek-ai/DeepSeek-V3-1',
      name: 'DeepSeek V3.1',
      api: 'openai-completions',
      provider: 'together',
      reasoning: true,
      input: ['text'],
      costInput: 0.6,
      costOutput: 1.7,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 131072,
    },
  ],
  minimax: [
    {
      id: 'MiniMax-M2.7',
      name: 'MiniMax-M2.7',
      api: 'anthropic-messages',
      provider: 'minimax',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.06,
      costCacheWrite: 0.375,
      contextWindow: 204800,
      maxTokens: 131072,
    },
    {
      id: 'MiniMax-M2.7-highspeed',
      name: 'MiniMax-M2.7 HighSpeed',
      api: 'anthropic-messages',
      provider: 'minimax',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.06,
      costCacheWrite: 0.375,
      contextWindow: 204800,
      maxTokens: 131072,
    },
  ],
  moonshotai: [
    {
      id: 'kimi-k2.6',
      name: 'Kimi K2.6',
      api: 'openai-completions',
      provider: 'moonshotai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.16,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 262144,
    },
    {
      id: 'kimi-k2.5',
      name: 'Kimi K2.5',
      api: 'openai-completions',
      provider: 'moonshotai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.1,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 262144,
    },
    {
      id: 'kimi-k2-thinking',
      name: 'Kimi K2 Thinking',
      api: 'openai-completions',
      provider: 'moonshotai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.15,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 262144,
    },
  ],
  'kimi-coding': [
    {
      id: 'k2p6',
      name: 'Kimi K2.6',
      api: 'anthropic-messages',
      provider: 'kimi-coding',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 32768,
    },
    {
      id: 'kimi-for-coding',
      name: 'Kimi For Coding',
      api: 'anthropic-messages',
      provider: 'kimi-coding',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 32768,
    },
    {
      id: 'kimi-k2-thinking',
      name: 'Kimi K2 Thinking',
      api: 'anthropic-messages',
      provider: 'kimi-coding',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 262144,
      maxTokens: 32768,
    },
  ],
  zai: [
    {
      id: 'glm-5.1',
      name: 'GLM-5.1',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 131072,
    },
    {
      id: 'glm-5',
      name: 'GLM-5',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 1,
      costOutput: 3.2,
      costCacheRead: 0.2,
      costCacheWrite: 0,
      contextWindow: 202800,
      maxTokens: 131100,
    },
    {
      id: 'glm-5-turbo',
      name: 'GLM-5-Turbo',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 131072,
    },
    {
      id: 'glm-5v-turbo',
      name: 'GLM-5V-Turbo',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 131072,
    },
    {
      id: 'glm-4.7',
      name: 'GLM-4.7',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 204800,
      maxTokens: 131072,
    },
    {
      id: 'glm-4.7-flash',
      name: 'GLM-4.7 Flash',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0.07,
      costOutput: 0.4,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 131072,
    },
    {
      id: 'glm-4.7-flashx',
      name: 'GLM-4.7 FlashX',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0.06,
      costOutput: 0.4,
      costCacheRead: 0.01,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 128000,
    },
    {
      id: 'glm-4.6',
      name: 'GLM-4.6',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0.6,
      costOutput: 2.2,
      costCacheRead: 0.11,
      costCacheWrite: 0,
      contextWindow: 204800,
      maxTokens: 131072,
    },
    {
      id: 'glm-4.6v',
      name: 'GLM-4.6V',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0.3,
      costOutput: 0.9,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 128000,
      maxTokens: 32768,
    },
    {
      id: 'glm-4.5-air',
      name: 'GLM-4.5-Air',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 98304,
    },
    {
      id: 'glm-4.5',
      name: 'GLM-4.5',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0.6,
      costOutput: 2.2,
      costCacheRead: 0.11,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 98304,
    },
    {
      id: 'glm-4.5-flash',
      name: 'GLM-4.5 Flash',
      api: 'openai-completions',
      provider: 'zai',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 131072,
      maxTokens: 98304,
    },
  ],
  opencode: [
    {
      id: 'claude-sonnet-4-5',
      name: 'Claude Sonnet 4.5',
      api: 'anthropic-messages',
      provider: 'opencode',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.3,
      costCacheWrite: 3.75,
      contextWindow: 200000,
      maxTokens: 64000,
    },
    {
      id: 'claude-opus-4-5',
      name: 'Claude Opus 4.5',
      api: 'anthropic-messages',
      provider: 'opencode',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.5,
      costCacheWrite: 6.25,
      contextWindow: 200000,
      maxTokens: 64000,
    },
    {
      id: 'gpt-5.1-codex',
      name: 'GPT-5.1 Codex',
      api: 'openai-responses',
      provider: 'opencode',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.107,
      costCacheWrite: 0,
      contextWindow: 400000,
      maxTokens: 128000,
    },
    {
      id: 'gemini-3.1-pro',
      name: 'Gemini 3.1 Pro',
      api: 'google-generative-ai',
      provider: 'opencode',
      reasoning: true,
      input: ['text', 'image'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.2,
      costCacheWrite: 0,
      contextWindow: 1048576,
      maxTokens: 65536,
    },
    {
      id: 'glm-5.1',
      name: 'GLM-5.1',
      api: 'openai-completions',
      provider: 'opencode',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0.26,
      costCacheWrite: 0,
      contextWindow: 204800,
      maxTokens: 131072,
    },
    {
      id: 'big-pickle',
      name: 'Big Pickle',
      api: 'anthropic-messages',
      provider: 'opencode',
      reasoning: true,
      input: ['text'],
      costInput: 0,
      costOutput: 0,
      costCacheRead: 0,
      costCacheWrite: 0,
      contextWindow: 200000,
      maxTokens: 128000,
    },
  ],
}

// ── Hooks ────────────────────────────────────────────────────

/** Fetch the list of available providers with model counts. */
export function useProviders() {
  return useQuery({
    queryKey: ['engine', 'providers'],
    queryFn: async (): Promise<ProviderInfo[]> => {
      try {
        const res = await api.get<{ providers: ProviderInfo[] }>('/api/engine/providers')
        return res.providers
      } catch {
        // Fall back to static catalog
      }
      return PROVIDER_CATALOG.map((p) => ({
        ...p,
        modelCount: (MODEL_CATALOG[p.id] ?? []).length,
      }))
    },
    staleTime: 5 * 60 * 1000,
  })
}

/** Fetch models, optionally filtered by provider. When `provider` is null,
 *  returns models from ALL connected providers (RFC-032). */
export function useModels(provider: string | null) {
  return useQuery({
    queryKey: ['engine', 'models', provider ?? '__all__'],
    queryFn: async (): Promise<ModelInfo[]> => {
      if (provider) {
        try {
          const res = await api.get<ProviderModelsResponse>(
            `/api/engine/models?provider=${encodeURIComponent(provider)}`,
          )
          return res.models
        } catch {
          // Fall back to static catalog
        }
        return MODEL_CATALOG[provider] ?? []
      }
      // RFC-032: fetch models from all connected providers
      try {
        const res = await api.get<ProviderModelsResponse>('/api/engine/models')
        return res.models
      } catch {
        // Fall back to static catalog — merge all providers
        const all: ModelInfo[] = []
        const seen = new Set<string>()
        for (const models of Object.values(MODEL_CATALOG)) {
          for (const m of models) {
            if (!seen.has(m.id)) {
              seen.add(m.id)
              all.push(m)
            }
          }
        }
        return all
      }
    },
    enabled: true,
    staleTime: 5 * 60 * 1000,
  })
}

/** Fetch the current engine configuration (from /api/engine/config, includes routing). */
export function useEngineConfig() {
  return useQuery({
    queryKey: ['engine', 'config'],
    queryFn: () => api.get<EngineConfigResponse>('/api/engine/config'),
    staleTime: 30 * 1000,
  })
}

/** Set the default model. */
export function useSetModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (model: string) => api.put('/api/engine/model', { model_id: model }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
    },
  })
}

/** Set the Oxios Copilot model. Pass null to clear (fall back to default_model). */
export function useSetQuickAskModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (modelId: string | null) =>
      api.put('/api/engine/quick-ask-model', { model_id: modelId }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
    },
  })
}

/** Set the API key for a provider. */
export function useSetApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ provider, apiKey }: { provider: string; apiKey: string }) =>
      api.put('/api/engine/api-key', { provider, api_key: apiKey }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
      qc.invalidateQueries({ queryKey: ['engine', 'providers'] })
    },
  })
}

/** Delete a provider's API key entirely. */
export function useDeleteApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (provider: string) =>
      api.delete(`/api/engine/api-key?provider=${encodeURIComponent(provider)}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
      qc.invalidateQueries({ queryKey: ['engine', 'providers'] })
    },
  })
}

/** Set per-provider options. */
export function useSetProviderOptions() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({
      provider,
      options,
    }: {
      provider: string
      options: Record<string, unknown>
    }) => api.put('/api/engine/provider-options', { provider, options }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
    },
  })
}

// ── Routing hooks (RFC-011) ───────────────────────────────────

/** Extract routing config from the engine config response. */
export function useRoutingConfig() {
  const { data } = useEngineConfig()
  return {
    data: data?.routing as RoutingConfig | undefined,
  }
}

/** Update routing configuration. */
export function useSetRouting() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: Partial<RoutingConfig>) => api.put('/api/engine/routing', body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'config'] })
    },
  })
}
// ── Role routing hooks (RFC-032) ────────────────────────────────

export interface RoleRoutingResponse {
  roles: Record<string, string>
  count: number
}

/** Fetch the current role routing config (role → model ID). */
export function useRoles() {
  return useQuery({
    queryKey: ['engine', 'roles'],
    queryFn: async (): Promise<RoleRoutingResponse> => {
      return api.get<RoleRoutingResponse>('/api/engine/roles')
    },
    staleTime: 60 * 1000,
  })
}

/** Update role routing config (PUT /api/engine/roles). */
export function useSetRoles() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (roles: Record<string, string>) =>
      api.put<{ ok: boolean; roles: Record<string, string> }>('/api/engine/roles', { roles }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['engine', 'roles'] })
    },
  })
}

/** Fetch routing statistics (model usage counts + costs). */
export function useRoutingStats() {
  return useQuery<RoutingStats>({
    queryKey: ['engine', 'routing', 'stats'],
    queryFn: () => api.get('/api/engine/routing/stats'),
    refetchInterval: 30000,
    retry: 1,
  })
}

/** Fetch recent fallback history. */
export function useFallbackHistory(limit = 20) {
  return useQuery<FallbackHistoryResponse>({
    queryKey: ['engine', 'routing', 'fallbacks', limit],
    queryFn: () => api.get(`/api/engine/routing/fallbacks?limit=${limit}`),
  })
}
