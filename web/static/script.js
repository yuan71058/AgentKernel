
const { createApp, ref, computed, nextTick, onMounted } = Vue;

createApp({
  setup() {
    // WS 地址自适应：URL 参数 > 本地开发 > 生产环境（Fly.io）
    const params = new URLSearchParams(window.location.search);
    const customWs = params.get('ws') || '';
    const defaultWs = window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1'
      ? 'ws://localhost:9991/ws'
      : 'wss://agentkernel.fly.dev/ws';
    const wsUrl = ref(customWs || defaultWs);
    // WS 下拉选择器
    const wsDropdownOpen = ref(false);
    const wsCustomMode = ref(false);
    const wsPresets = [
      { label: '本地开发', value: 'ws://localhost:9991/ws' },
      { label: 'Fly.io 线上', value: 'wss://agentkernel.fly.dev/ws' },
    ];
    const wsSelectLabel = computed(() => {
      const found = wsPresets.find(p => p.value === wsUrl.value);
      return found ? found.label : '自定义';
    });
    function selectWsPreset(val) {
      wsUrl.value = val;
      wsDropdownOpen.value = false;
      wsCustomMode.value = false;
    }
    const currentSessionTitle = computed(() => {
      const found = sessions.value.find(s => s.session_id === sessionId.value);
      return found ? (found.title || found.session_id) : sessionId.value || '选择会话';
    });
    const canRetrySession = computed(() => {
      if (!connected.value || !sessionId.value) return false;
      if (currentRunId.value && ['pending','running','streaming','cancelling'].includes(currentRunStatus.value)) return false;
      for (let i = chatMessages.value.length - 1; i >= 0; i--) {
        const m = chatMessages.value[i];
        if (!m) continue;
        if (m.role === 'tool') return true;
        if (m.role === 'user') return true;
        if (m.role === 'assistant') return !!(m.toolCalls && m.toolCalls.length > 0);
      }
      return false;
    });
    // 点击外部关闭下拉
    const activeSessionMenu = ref(null);
    if (typeof document !== 'undefined') {
      document.addEventListener('click', (e) => {
        if (!e.target.closest('.ws-select')) wsDropdownOpen.value = false;
        if (!e.target.closest('.mobile-session-bar')) mobileSessionOpen.value = false;
        if (!e.target.closest('.mobile-more-popup') && !e.target.closest('.btn-more')) mobileMoreOpen.value = false;
        if (!e.target.closest('.session-action-menu') && !e.target.closest('.btn-session-menu')) activeSessionMenu.value = null;
      });
    }
    const connected = ref(false);
    const connectionId = ref('');
    const sessionId = ref('debug_session');
    const chatInput = ref('');
    const chatMessages = ref([]);
    const pendingImages = ref([]); // [{id, name, dataUrl, base64, mediaType}]
    const pendingAudio = ref([]); // [{id, name, dataUrl, base64, format, duration}]
    const isRecording = ref(false);
    let _mediaRecorder = null;
    let _recordedChunks = [];
    const sessions = ref([]);
    const archivedSessions = ref([]);
    const showArchivedSessions = ref(false);
    const selectedSessionId = ref('');
    const events = ref([]);
    const rawMessages = ref([]);
    const tools = ref([]);
    const rightTab = ref('summary');
    const statsExpanded = ref(false);
    const eventFilter = ref('all');
    const rawFilter = ref('all');
    const rawInput = ref('');
    const fullMessages = ref([]);
    const fullMessagesPage = ref(0);
    const fullMessagesTotal = ref(0);
    const fullMessagesHasMore = ref(true);
    const fullMessagesLoading = ref(false);
    const runtimeSessions = ref([]);
    const runtimeRuns = ref([]);
    const runtimeRuntimeStatus = ref('');
    const runtimeLoading = ref(false);
    const latestToolChainReport = ref(null);
    const latestTraceDetails = ref([]);

    const mobileView = ref('chat'); // 'chat' | 'debug'
    const mobileSessionOpen = ref(false);
    const mobileMoreOpen = ref(false);

    const chatBody = ref(null);
    const streamBody = ref(null);
    const rawBody = ref(null);
    const fullMessagesBody = ref(null);

    const providerConfig = ref({
      protocol: 'openai',
      base_url: 'https://api.deepseek.com',
      api_key: '',
      model: 'deepseek-chat',
      max_tokens: 4096,
      temperature: 0,
      supports_image: false,
      supports_audio: false,
    });
    const providerStatus = ref('');
    const systemPrompt = ref('');
    const systemPromptStatus = ref('');
    let _systemPromptAutoApplied = false;
    const contextPreview = ref({});
    const contextStatus = ref('');
    const contextTrimMode = ref('checkpoint');
    const contextKeepMessages = ref(50);
    const contextTriggerMaxMessages = ref(300);
    const contextRetainTurns = ref(20);
    const contextIncludeAfter = ref('');
    const contextSeedKind = ref('system_memory');
    const contextSeedContent = ref('');
    const contextSeedMode = ref('add');

    const showPresetToolsModal = ref(false);
    const showApiKey = ref(false);

    const askUserQuestionState = ref(null);
    const pendingCommandWaiters = new Map();

    const AGENT_TOOL_PROVIDER_TEMPLATE = Object.freeze({
      protocol: 'openai',
      base_url: 'https://api.deepseek.com',
      api_key: '',
      model: 'deepseek-reasoner',
      max_tokens: 4096,
      temperature: 0.5,
    });
    const AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE = '你是一个子agent，一次性对话的，不具备连续对话，所以你不能询问用户，如果真的要询问，必须让用户重新详细描述，而非”补充”';

    // 默认系统提示词（自动应用于无自定义提示词的 session）
    const DEFAULT_SYSTEM_PROMPT = `你是 AgentKernel 调试台的 AI 助手。

你的运行环境：
- 项目：AgentKernel — 基于 Rust 的 AI Runtime Kernel
- 项目主页：https://github.com/cih1996/AgentKernel
- 在线体验：https://cih1996.github.io/AgentKernel/
- 架构定位：后端内部的 AI 运行时内核，不是对外 API 服务

你的能力：
- 你拥有内置工具（get_time、calc、fetch_url 等），可以主动使用它们完成任务
- fetch_url 工具可以访问互联网资源，如获取项目 README、文档等
- 你可以用 fetch_url 获取项目最新状态：https://raw.githubusercontent.com/cih1996/AgentKernel/refs/heads/main/README.md

行为准则：
- 用户问关于项目的问题时，优先用 fetch_url 工具获取最新信息，不要只靠训练数据回答
- 用中文回答
- 简洁直接，不废话`;

    // 流式文本累积
    const streamingText = ref('');
    const streamingThinking = ref('');
    const isStreaming = ref(false);
    const currentRunId = ref('');
    const currentRunStatus = ref('idle');

    // 自动重连
    const autoReconnect = ref(true);
    const reconnectTimer = ref(null);

    let ws = null;
    let reqCounter = 0;
    // 记录发出的命令类型和所属 session（request_id → { command, sessionId }），用于响应回填隔离
    const pendingCommands = new Map();

    // ─── 预置工具定义 ──────────────────────────────────────
    const presetTools = [
      {
        name: 'get_time',
        description: '获取当前时间（北京时间 UTC+8）',
        schema: { type: 'object', properties: { format: { type: 'string', description: '输出格式: iso / locale / timestamp', default: 'locale' } } },
        tags: ['builtin', 'time'],
        execute(input) {
          const fmt = input.format || 'locale';
          const now = new Date();
          if (fmt === 'timestamp') return { ok: true, result: String(Date.now()) };
          if (fmt === 'iso') return { ok: true, result: now.toISOString() };
          return { ok: true, result: now.toLocaleString('zh-CN', { timeZone: 'Asia/Shanghai', hour12: false }) };
        }
      },
      {
        name: 'echo',
        description: '回显输入内容（调试用）',
        schema: { type: 'object', properties: { text: { type: 'string', description: '要回显的文本' } }, required: ['text'] },
        tags: ['builtin', 'debug'],
        execute(input) { return { ok: true, result: input.text || '' }; }
      },
      {
        name: 'calc',
        description: '简单数学计算（支持 + - * / % **）',
        schema: { type: 'object', properties: { expression: { type: 'string', description: '数学表达式，如 "2 + 3 * 4"' } }, required: ['expression'] },
        tags: ['builtin', 'math'],
        execute(input) {
          const expr = (input.expression || '').replace(/[^0-9+\-*/.%()\s]/g, '');
          if (!expr) return { ok: false, result: '无效表达式' };
          try {
            const val = Function('"use strict"; return (' + expr + ')')();
            return { ok: true, result: String(val) };
          } catch(e) { return { ok: false, result: '计算错误: ' + e.message }; }
        }
      },
      {
        name: 'random_number',
        description: '生成随机整数',
        schema: { type: 'object', properties: { min: { type: 'integer', default: 1 }, max: { type: 'integer', default: 100 } } },
        tags: ['builtin', 'random'],
        execute(input) {
          const min = Math.ceil(input.min ?? 1);
          const max = Math.floor(input.max ?? 100);
          const val = Math.floor(Math.random() * (max - min + 1)) + min;
          return { ok: true, result: String(val) };
        }
      },
      {
        name: 'generate_uuid',
        description: '生成 UUID v4',
        schema: { type: 'object', properties: {} },
        tags: ['builtin', 'util'],
        execute() {
          const uuid = ([1e7]+-1e3+-4e3+-8e3+-1e11).replace(/[018]/g, c =>
            (c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> c / 4).toString(16));
          return { ok: true, result: uuid };
        }
      },
      {
        name: 'json_format',
        description: '格式化 JSON 字符串',
        schema: { type: 'object', properties: { json: { type: 'string', description: 'JSON 字符串' }, indent: { type: 'integer', default: 2 } }, required: ['json'] },
        tags: ['builtin', 'util'],
        execute(input) {
          try {
            const obj = JSON.parse(input.json);
            return { ok: true, result: JSON.stringify(obj, null, input.indent ?? 2) };
          } catch(e) { return { ok: false, result: 'JSON 解析错误: ' + e.message }; }
        }
      },
      {
        name: 'word_count',
        description: '统计文本字数/词数/行数',
        schema: { type: 'object', properties: { text: { type: 'string', description: '要统计的文本' } }, required: ['text'] },
        tags: ['builtin', 'text'],
        execute(input) {
          const text = input.text || '';
          const chars = text.length;
          const words = text.trim() ? text.trim().split(/\s+/).length : 0;
          const lines = text.split('\n').length;
          return { ok: true, result: `字符: ${chars}, 词: ${words}, 行: ${lines}` };
        }
      },
      {
        name: 'base64',
        description: 'Base64 编码/解码',
        schema: { type: 'object', properties: { text: { type: 'string' }, action: { type: 'string', enum: ['encode', 'decode'], default: 'encode' } }, required: ['text'] },
        tags: ['builtin', 'encoding'],
        execute(input) {
          try {
            if (input.action === 'decode') return { ok: true, result: atob(input.text) };
            return { ok: true, result: btoa(input.text) };
          } catch(e) { return { ok: false, result: e.message }; }
        }
      },
      {
        name: 'fetch_url',
        description: '通过 HTTP GET 获取指定 URL 的内容（纯文本）。支持 raw.githubusercontent.com 等无跨域限制的资源。可用于获取项目文档、JSON 数据、公开 API 等。',
        schema: {
          type: 'object',
          properties: {
            url: { type: 'string', description: '要获取的 URL 地址' },
            max_length: { type: 'integer', description: '返回内容最大字符数，默认 8000', default: 8000 }
          },
          required: ['url']
        },
        tags: ['builtin', 'network'],
        async execute(input) {
          const url = String(input.url || '').trim();
          if (!url) return { ok: false, result: '缺少 url 参数' };
          const maxLength = input.max_length || 8000;
          try {
            const controller = new AbortController();
            const timer = setTimeout(() => controller.abort(), 15000);
            const resp = await fetch(url, {
              signal: controller.signal,
              headers: { 'Accept': 'text/plain, text/html, application/json, */*' }
            });
            clearTimeout(timer);
            if (!resp.ok) {
              return { ok: false, result: `HTTP ${resp.status} ${resp.statusText} — 请求失败: ${url}` };
            }
            let text = await resp.text();
            let truncated = false;
            if (text.length > maxLength) {
              text = text.slice(0, maxLength);
              truncated = true;
            }
            const suffix = truncated ? `\n\n[内容已截断，原始长度超过 ${maxLength} 字符]` : '';
            return { ok: true, result: text + suffix };
          } catch (e) {
            const msg = e.name === 'AbortError'
              ? `请求超时（15秒）: ${url}`
              : `请求失败: ${e.message}`;
            return { ok: false, result: msg };
          }
        }
      },
      {
        name: 'AskUserQuestion',
        description: '向用户提问并等待用户回答（阻塞交互）。用于需要用户决策或补充信息的场景。',
        schema: {
          type: 'object',
          properties: {
            question: { type: 'string', description: '要向用户提出的问题' },
            options: { type: 'array', items: { type: 'string' }, description: '可选的回答选项列表。若提供则显示为单选按钮，若不提供则显示文本输入框。' }
          },
          required: ['question']
        },
        tags: ['builtin', 'interactive'],
        execute(input) {
          return new Promise((resolve) => {
            askUserQuestionState.value = {
              question: input.question || '（未提供问题）',
              options: input.options || [],
              answer: '',
              resolve: (answer) => {
                askUserQuestionState.value = null;
                resolve({ ok: true, result: String(answer) });
              }
            };
          });
        }
      },
      {
        name: 'agent',
        description: '创建一个临时子会话，让 AI 调用 AI 完成一次独立对话，并把最终结果回传给当前工具调用。',
        schema: {
          type: 'object',
          properties: {
            message: { type: 'string', description: '要发给子 Agent 的消息内容' }
          },
          required: ['message']
        },
        tags: ['builtin', 'agent'],
        async execute(input) {
          const message = String(input?.message || '').trim();
          if (!message) {
            return { ok: false, result: 'agent 工具缺少 message 参数' };
          }
          return runNestedAgent(message);
        }
      },
    ];

    // 工具注册表（name → tool 定义）
    const toolRegistry = new Map();
    presetTools.forEach(t => toolRegistry.set(t.name, t));

    // Markdown 渲染函数
    function renderMarkdown(text) {
      if (!text) return '';
      try {
        const rawHtml = marked.parse(text);
        return DOMPurify.sanitize(rawHtml);
      } catch (e) {
        return text;
      }
    }

    /**
     * 注册本地工具处理器。
     * 业务端扩展时只需要关注 execute(input, ctx)，其他 call_id / WS 回包 / 日志由统一入口处理。
     */
    function defineClientTool(definition) {
      if (!definition || !definition.name || typeof definition.execute !== 'function') {
        throw new Error('defineClientTool requires { name, execute }');
      }
      const tool = {
        description: '',
        schema: { type: 'object', properties: {} },
        tags: [],
        ...definition,
      };
      toolRegistry.set(tool.name, tool);
      return tool;
    }

    function normalizeToolExecutionResult(value) {
      if (value && typeof value === 'object' && Object.prototype.hasOwnProperty.call(value, 'ok')) {
        return {
          ok: value.ok !== false,
          result: value.result == null ? '' : String(value.result),
        };
      }
      return { ok: true, result: value == null ? '' : String(value) };
    }

    // 工具执行日志
    const toolExecLog = ref([]);

    function registerPresetTool(t) {
      if (!ws || !t) return;
      const rid = nextReqId();
      const msg = {
        command: 'tool.register',
        request_id: rid,
        session_id: sessionId.value,
        payload: {
          tool_name: t.name,
          description: t.description,
          schema: t.schema,
          client_id: 'web_debug',
          timeout_ms: 0,
          tags: t.tags,
        }
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
      addLocalNotice('工具注册', `已发送注册请求: ${t.name}`);
    }

    function registerPresetTools() {
      if (!ws) return;
      presetTools.forEach(t => registerPresetTool(t));
    }

    function nextReqId() { return 'r' + (++reqCounter); }
    function now() { return new Date().toLocaleTimeString('zh-CN', { hour12: false }); }

    function runStatusLabel(status) {
      const map = {
        idle: '空闲',
        pending: '已发送',
        running: '运行中',
        streaming: '生成中',
        cancelling: '取消中',
        cancelled: '已中断',
        completed: '已完成',
        failed: '失败',
      };
      return map[status] || status || '未知';
    }

    function formatSize(str) {
      if (str == null) return '0 B';
      const bytes = new Blob([String(str)]).size;
      if (bytes < 1024) return bytes + ' B';
      return (bytes / 1024).toFixed(2) + ' KB';
    }

    function switchMobileView(view) {
      mobileView.value = view;
      const rightPanel = document.querySelector('.right');
      if (rightPanel) {
        if (view === 'debug') {
          rightPanel.classList.add('mobile-show');
        } else {
          rightPanel.classList.remove('mobile-show');
        }
      }
    }

    function formatTimeoutLabel(timeoutMs) {
      if (timeoutMs == null) return '未设置';
      if (timeoutMs === 0) return '无限';
      return `${timeoutMs}ms`;
    }

    function formatToolChainIds(ids) {
      return Array.isArray(ids) && ids.length ? ids.join(', ') : '—';
    }

    function isAgentToolConfigured() {
      const provider = AGENT_TOOL_PROVIDER_TEMPLATE;
      const providerReady = [provider.base_url, provider.api_key, provider.model].every(v => typeof v === 'string' && !v.startsWith('__TODO_'));
      const promptReady = typeof AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE === 'string' && !AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE.startsWith('__TODO_');
      return providerReady && promptReady;
    }

    function createEphemeralSessionId(prefix = 'agent') {
      return `${prefix}_${Math.random().toString(36).slice(2, 10)}`;
    }

    function getAgentDelegatedTools() {
      return [...toolRegistry.values()].filter(tool => tool && tool.name && tool.name !== 'agent');
    }

    function toolChainIssueCount(report) {
      if (!report) return 0;
      return (report.dropped_incomplete_tool_call_ids?.length || 0) + (report.dropped_orphan_tool_result_ids?.length || 0);
    }

    function toolChainStatusText(report) {
      if (!report) return '暂无诊断';
      const issues = toolChainIssueCount(report);
      if (issues > 0) return `发现 ${issues} 个对齐问题`;
      if ((report.complete_tool_call_ids?.length || 0) > 0) return '工具链完整';
      return '本轮没有工具链问题';
    }

    function getPendingMeta(requestId) {
      const meta = pendingCommands.get(requestId);
      if (!meta) return { command: '', sessionId: '' };
      if (typeof meta === 'string') return { command: meta, sessionId: '' };
      return {
        command: meta.command || '',
        sessionId: meta.sessionId || '',
      };
    }

    function isCurrentSession(targetSessionId) {
      if (!targetSessionId) return true;
      return targetSessionId === sessionId.value;
    }

    function isTerminalRunStatus(status) {
      return ['idle', 'completed', 'cancelled', 'failed'].includes(status);
    }

    function appendAssistantMessage(text, meta = now()) {
      const normalized = String(text || '').trim();
      if (!normalized) return false;
      const lastMsg = chatMessages.value[chatMessages.value.length - 1];
      if (lastMsg && lastMsg.role === 'assistant' && String(lastMsg.text || '').trim() === normalized) {
        if (meta) lastMsg.meta = meta;
        return false;
      }
      if (lastMsg && lastMsg.role === 'assistant' && lastMsg._toolChainGroup && lastMsg.runId === currentRunId.value) {
        lastMsg.text = normalized;
        if (meta) lastMsg.meta = meta;
        return true;
      }
      chatMessages.value.push({ role: 'assistant', text: normalized, meta, runId: currentRunId.value || '' });
      return true;
    }

    function normalizeToolResultContent(value) {
      if (value == null) return '';
      if (typeof value === 'string') return value;
      try { return JSON.stringify(value, null, 2); } catch { return String(value); }
    }

    function getMessageRunId(message) {
      return message?.run_id || message?.metadata?.run_id || '';
    }

    function getOrCreateToolChainGroup(mappedMessages, runId, meta) {
      for (let i = mappedMessages.length - 1; i >= 0; i--) {
        const item = mappedMessages[i];
        if (item && item.role === 'assistant' && item._toolChainGroup && item.runId === runId) {
          return item;
        }
        if (item && item.runId && runId && item.runId !== runId) break;
      }
      const group = {
        role: 'assistant',
        text: '',
        meta,
        runId,
        _toolChainGroup: true,
        _toolChainExpanded: false,
        toolCalls: [],
      };
      mappedMessages.push(group);
      return group;
    }

    function attachToolResultToGroups(mappedMessages, tr, meta, runId) {
      const result = normalizeToolResultContent(tr.content);
      for (let j = mappedMessages.length - 1; j >= 0; j--) {
        const prev = mappedMessages[j];
        if (!prev.toolCalls || (runId && prev.runId && prev.runId !== runId)) continue;
        const tc = prev.toolCalls.find(t => t.id === tr.tool_use_id);
        if (tc) {
          tc.result = result;
          tc.isError = !!tr.is_error;
          tc.status = tr.is_error ? 'error' : 'done';
          return true;
        }
      }
      mappedMessages.push({
        role: 'tool',
        text: result,
        meta,
        runId,
        toolResult: { toolUseId: tr.tool_use_id, isError: !!tr.is_error, status: 'orphan_result' },
      });
      return false;
    }

    function mapProtocolMessagesToChatMessages(messages) {
      const mappedMessages = [];
      for (const m of messages || []) {
        if (m.role !== 'user' && m.role !== 'assistant' && m.role !== 'tool') continue;
        const meta = m.created_at ? new Date(m.created_at).toLocaleTimeString('zh-CN', {hour12:false}) : '';
        const runId = getMessageRunId(m);
        const content = Array.isArray(m.content) ? m.content : [];

        const toolUses = content.filter(c => c && c.type === 'tool_use');
        const toolResults = content.filter(c => c && c.type === 'tool_result');
        const textBlocks = content.filter(c => c && c.type === 'text');
        const imageBlocks = content.filter(c => c && c.type === 'image');
        const audioBlocks = content.filter(c => c && c.type === 'audio');
        const textContent = textBlocks.map(c => c.text || '').join('').trim() || m.text || '';

        const historyImages = imageBlocks.map(ib => {
          const src = ib.source || {};
          const mediaType = src.media_type || 'image/png';
          const data = src.data || '';
          return { name: 'image', dataUrl: `data:${mediaType};base64,${data}` };
        });

        const historyAudio = audioBlocks.map(ab => {
          const src = ab.source || {};
          const format = src.format || 'wav';
          const data = src.data || '';
          const mimeMap = { wav: 'audio/wav', mp3: 'audio/mpeg', ogg: 'audio/ogg', webm: 'audio/webm', mp4: 'audio/mp4' };
          const mime = mimeMap[format] || 'audio/wav';
          return { name: `audio.${format}`, dataUrl: `data:${mime};base64,${data}`, format, duration: 0 };
        });

        if (toolUses.length > 0) {
          const group = getOrCreateToolChainGroup(mappedMessages, runId, meta);
          if (textContent) {
            group.text = group.text ? `${group.text}\n${textContent}` : textContent;
          }
          group.meta = meta || group.meta;
          for (const tu of toolUses) {
            if (!group.toolCalls.some(t => t.id === tu.id)) {
              group.toolCalls.push({
                id: tu.id,
                name: tu.name,
                input: tu.input || {},
                result: null,
                isError: false,
                status: 'pending',
              });
            }
          }
        }

        if (toolResults.length > 0) {
          for (const tr of toolResults) {
            attachToolResultToGroups(mappedMessages, tr, meta, runId);
          }
        }

        if (toolUses.length === 0 && toolResults.length === 0 && (textContent || historyImages.length > 0 || historyAudio.length > 0)) {
          if (m.role === 'assistant' && runId) {
            const group = getOrCreateToolChainGroup(mappedMessages, runId, meta);
            group.text = group.text ? `${group.text}\n${textContent}` : textContent;
            group.meta = meta || group.meta;
            if (historyImages.length > 0) group.images = [...(group.images || []), ...historyImages];
            if (historyAudio.length > 0) group.audioFiles = [...(group.audioFiles || []), ...historyAudio];
          } else {
            const entry = { role: m.role, text: textContent, meta, runId };
            if (historyImages.length > 0) entry.images = historyImages;
            if (historyAudio.length > 0) entry.audioFiles = historyAudio;
            mappedMessages.push(entry);
          }
        }
      }
      return mappedMessages;
    }

    function ensureActiveToolChainMessage(runId = currentRunId.value) {
      let lastMsg = chatMessages.value[chatMessages.value.length - 1];
      if (!lastMsg || lastMsg.role !== 'assistant' || !lastMsg.toolCalls || (runId && lastMsg.runId && lastMsg.runId !== runId)) {
        lastMsg = { role: 'assistant', text: '', runId, _toolChainGroup: true, _toolChainExpanded: true, toolCalls: [] };
        chatMessages.value.push(lastMsg);
      }
      if (!lastMsg.toolCalls) lastMsg.toolCalls = [];
      lastMsg._toolChainGroup = true;
      lastMsg._toolChainExpanded = true;
      if (runId && !lastMsg.runId) lastMsg.runId = runId;
      return lastMsg;
    }

    function toolChainSummary(toolCalls) {
      const calls = Array.isArray(toolCalls) ? toolCalls : [];
      const total = calls.length;
      const error = calls.filter(t => t.status === 'error').length;
      const pending = calls.filter(t => t.status === 'pending').length;
      const done = calls.filter(t => t.status === 'done').length;
      if (error > 0) return `工具链 ${total} 个：${done} 完成 / ${error} 失败 / ${pending} 执行中`;
      if (pending > 0) return `工具链 ${total} 个：${done} 完成 / ${pending} 执行中`;
      return `工具链 ${total} 个：全部完成`;
    }

    function toolChainStatus(toolCalls) {
      const calls = Array.isArray(toolCalls) ? toolCalls : [];
      if (calls.some(t => t.status === 'error')) return 'error';
      if (calls.some(t => t.status === 'pending')) return 'pending';
      return calls.length ? 'done' : 'idle';
    }

    function connect() {
      if (ws) ws.close();
      if (reconnectTimer.value) { clearTimeout(reconnectTimer.value); reconnectTimer.value = null; }
      wsDropdownOpen.value = false;
      // HTTPS 页面自动修正 ws:// → wss://
      let url = wsUrl.value;
      if (window.location.protocol === 'https:' && url.startsWith('ws://')) {
        url = url.replace('ws://', 'wss://');
        wsUrl.value = url;
      }
      try { ws = new WebSocket(url); } catch(e) { addLocalNotice('连接失败', e.message); scheduleReconnect(); return; }

      ws.onopen = () => {
        connected.value = true;
        addLocalNotice('WS', '连接建立');
        // 连接后只查询当前 session 状态，不自动注册工具
        loadSessions();
        loadSessionHistory();
        loadProvider();
        loadSystemPrompt();
        loadTools();
        loadContextPreview();
        loadRuntimeSessions();
      };
      ws.onclose = () => {
        pendingCommandWaiters.forEach(waiter => {
          waiter.reject(new Error('WS 连接已断开'));
        });
        pendingCommandWaiters.clear();
        connected.value = false;
        connectionId.value = '';
        currentRunId.value = '';
        currentRunStatus.value = 'idle';
        streamingText.value = '';
        streamingThinking.value = '';
        isStreaming.value = false;
        isStreaming.value = false;
        runtimeSessions.value = [];
        runtimeRuns.value = [];
        runtimeRuntimeStatus.value = '';
        runtimeLoading.value = false;
        latestToolChainReport.value = null;
        latestTraceDetails.value = [];
        addLocalNotice('WS', '连接断开');
        if (autoReconnect.value) scheduleReconnect();
      };
      ws.onerror = () => {
        addLocalNotice('WS', '连接错误');
      };
      ws.onmessage = (evt) => {
        const raw = evt.data;
        addRawMessage('in', raw);

        let msg;
        try { msg = JSON.parse(raw); } catch { return; }

        // Hello response
        if (msg.type === 'response' && msg.request_id === 'hello') {
          connectionId.value = msg.payload?.connection_id || '';
          return;
        }

        // Event
        if (msg.type === 'event') {
          const e = msg;
          const type = e.event_type || 'unknown';
          const payload = e.payload || {};
          addEvent(type, e.session_id, payload, e.run_id, e);
          handleEvent(type, payload, e);
          return;
        }

        // Response
        if (msg.type === 'response') {
          // 先刷入正在流式显示的文本（防止 model.completed 还没到时 response 先到导致丢失）
          flushStreaming();

          const pendingMeta = getPendingMeta(msg.request_id);
          const respCmd = pendingMeta.command;
          const respSessionId = msg.payload?.session_id || pendingMeta.sessionId || '';

          if (!msg.success) {
            addLocalNotice('命令失败', `[${msg.request_id}] ${msg.payload?.error || '未知错误'}`);
            if (respCmd === 'session.send' && isCurrentSession(respSessionId)) {
              currentRunStatus.value = 'failed';
              streamingText.value = '';
              streamingThinking.value = '';
              isStreaming.value = false;
              loadRuntimeSessions();
            }
          }

          // session.send 的响应不再 push 到聊天（assistant 消息已通过 model.completed 显示）
          // 其他 command 的响应照常显示为 system 消息

          // provider.get 响应 → 回填配置表单
          if (msg.success && msg.request_id.startsWith('r') && msg.payload?.provider && isCurrentSession(respSessionId)) {
            const p = msg.payload.provider;
            providerConfig.value.protocol = p.protocol || 'openai';
            providerConfig.value.base_url = p.base_url || '';
            providerConfig.value.api_key = p.api_key || '';
            providerConfig.value.model = p.model || '';
            providerConfig.value.max_tokens = p.max_tokens || 4096;
            providerConfig.value.temperature = p.temperature || 0;
            providerConfig.value.supports_image = !!p.supports_image;
            providerConfig.value.supports_audio = !!p.supports_audio;
            providerStatus.value = `已读取 (${msg.payload.is_override ? 'Session 覆盖' : '全局默认'})`;
            setTimeout(() => providerStatus.value = '', 4000);
          }
          // provider.update 响应
          if (msg.success && msg.payload?.provider && msg.request_id.startsWith('r') && isCurrentSession(respSessionId)) {
            const p = msg.payload.provider;
            providerStatus.value = `已保存: ${p.protocol} / ${p.model}`;
            setTimeout(() => providerStatus.value = '', 4000);
          }
          // system_prompt.get / set 响应
          if (msg.success && Object.prototype.hasOwnProperty.call(msg.payload || {}, 'system_prompt') && isCurrentSession(respSessionId)) {
            systemPrompt.value = msg.payload.system_prompt || '';
            systemPromptStatus.value = msg.payload.updated
              ? `已保存 (${msg.payload.is_session_override ? 'Session 覆盖' : '全局默认'})`
              : `已读取 (${msg.payload.is_session_override ? 'Session 覆盖' : '全局默认'})`;
            setTimeout(() => systemPromptStatus.value = '', 4000);
            // 自动设置默认提示词：session 无自定义提示词且尚未自动设置过
            if (!msg.payload.updated && !msg.payload.is_session_override && !systemPrompt.value && !_systemPromptAutoApplied) {
              _systemPromptAutoApplied = true;
              systemPrompt.value = DEFAULT_SYSTEM_PROMPT;
              const autoRid = nextReqId();
              const autoMsg = {
                command: 'system_prompt.set',
                request_id: autoRid,
                session_id: respSessionId,
                payload: { system_prompt: DEFAULT_SYSTEM_PROMPT },
              };
              const autoOut = JSON.stringify(autoMsg);
              ws.send(autoOut);
              addRawMessage('out', autoOut);
            }
          }
          if (respCmd === 'session.messages.full') {
            fullMessagesLoading.value = false;
          }
          if (respCmd === 'runtime.sessions') {
            runtimeLoading.value = false;
          }
          if (msg.success && respCmd === 'run.cancel' && isCurrentSession(respSessionId)) {
            currentRunStatus.value = msg.payload?.status === 'cancelling' ? 'cancelling' : currentRunStatus.value;
            if (msg.payload?.run_id) currentRunId.value = msg.payload.run_id;
            loadRuntimeSessions();
            addLocalNotice(
              '推理',
              msg.payload?.cancelled
                ? `已请求中断 run: ${msg.payload.run_id || '未知'}`
                : `未找到可中断的 run: ${msg.payload?.run_id || '未知'}`
            );
          }
          if (msg.success && msg.payload?.active_context && isCurrentSession(respSessionId)) {
            contextPreview.value = {
              ...contextPreview.value,
              ...msg.payload,
              active_context: msg.payload.active_context,
            };
            const actionLabel = respCmd && respCmd.startsWith('context.')
              ? contextCommandName(respCmd)
              : '上下文已更新';
            contextStatus.value = `${actionLabel}：${contextModeLabel(msg.payload.active_context.mode)}`;
            setTimeout(() => contextStatus.value = '', 4000);
            if (respCmd && respCmd.startsWith('context.') && respCmd !== 'context.preview') {
              loadContextPreview();
            }
          }
          if (msg.success && msg.payload?.seed && respCmd === 'context.seed.add' && isCurrentSession(respSessionId)) {
            contextSeedContent.value = '';
            loadContextPreview();
          }
          if (msg.success && msg.payload?.seed && respCmd === 'context.seed.set' && isCurrentSession(respSessionId)) {
            contextSeedContent.value = '';
            loadContextPreview();
          }
          if (msg.success && respCmd === 'context.seed.delete' && isCurrentSession(respSessionId)) {
            loadContextPreview();
          }
          if (msg.success && respCmd === 'context.seed.clear' && isCurrentSession(respSessionId)) {
            loadContextPreview();
          }

          // tool.list 响应 → 回填工具列表
          if (msg.success && Array.isArray(msg.payload?.tools) && isCurrentSession(respSessionId)) {
            tools.value = msg.payload.tools.map(t => ({
              name: t.name,
              description: t.description,
              client: t.client_id || '—',
              timeout: formatTimeoutLabel(t.timeout_ms),
              tags: t.tags || [],
              input_schema: t.input_schema || {},
            }));
          }

          if (msg.success && respCmd === 'runtime.sessions') {
            runtimeSessions.value = Array.isArray(msg.payload?.sessions) ? msg.payload.sessions : [];
            runtimeRuns.value = Array.isArray(msg.payload?.runs) ? msg.payload.runs : [];
            runtimeRuntimeStatus.value = `已刷新 ${now()}`;

            const activeTrackedRun = currentRunId.value
              ? runtimeRuns.value.find(r => r.run_id === currentRunId.value)
              : null;
            const fallbackSessionRun = !currentRunId.value
              ? runtimeRuns.value.find(r => r.session_id === sessionId.value)
              : null;

            if (activeTrackedRun) {
              if (activeTrackedRun.status === 'cancelling') {
                currentRunStatus.value = 'cancelling';
              } else if (isTerminalRunStatus(currentRunStatus.value)) {
                currentRunStatus.value = activeTrackedRun.status;
              }
            } else if (fallbackSessionRun) {
              currentRunId.value = fallbackSessionRun.run_id;
              currentRunStatus.value = fallbackSessionRun.status;
            } else {
              if (['pending', 'running', 'streaming', 'cancelling'].includes(currentRunStatus.value)) {
                currentRunStatus.value = 'idle';
                currentRunId.value = '';
              }
            }
          }

          // tool.register / unregister 响应后刷新 session 工具配置
          if (msg.success && (respCmd === 'tool.register' || respCmd === 'tool.unregister')) {
            loadTools();
          }

          // session.list 响应 → 回填 session 下拉
          if (msg.success && respCmd === 'session.list' && Array.isArray(msg.payload?.sessions)) {
            const list = msg.payload.sessions.filter(item => item && typeof item === 'object' && item.session_id);
            if (msg.payload?.status === 'archived') {
              archivedSessions.value = list;
            } else {
              sessions.value = list;
              selectedSessionId.value = sessionId.value;
            }
          }
          if (msg.success && respCmd === 'session.close') {
            const closedSessionId = msg.payload?.session_id || respSessionId;
            addLocalNotice('会话', `已关闭会话: ${closedSessionId}（历史已保留）`);
            loadSessions();
            loadRuntimeSessions();
          }
          if (msg.success && respCmd === 'session.archive') {
            const archivedSessionId = msg.payload?.session_id || respSessionId;
            removeSessionFromVisibleLists(archivedSessionId);
            if (archivedSessionId === sessionId.value) switchToFirstAvailableSession();
            addLocalNotice('会话', `已归档会话: ${archivedSessionId}`);
            loadSessions();
            if (showArchivedSessions.value) loadArchivedSessions();
          }
          if (msg.success && respCmd === 'session.unarchive') {
            const unarchivedSessionId = msg.payload?.session_id || respSessionId;
            archivedSessions.value = archivedSessions.value.filter(item => item.session_id !== unarchivedSessionId);
            addLocalNotice('会话', `已恢复归档会话: ${unarchivedSessionId}`);
            loadSessions();
            if (showArchivedSessions.value) loadArchivedSessions();
          }
          if (msg.success && respCmd === 'session.delete') {
            const deletedSessionId = msg.payload?.session_id || respSessionId;
            removeSessionFromVisibleLists(deletedSessionId);
            if (deletedSessionId === sessionId.value) switchToFirstAvailableSession();
            addLocalNotice('会话', `已永久删除会话: ${deletedSessionId}`);
            loadSessions();
            if (showArchivedSessions.value) loadArchivedSessions();
          }

          // session.fork 响应 → 刷新 session 列表并切换到新 session
          if (msg.success && respCmd === 'session.fork') {
            const newId = msg.payload?.new_session_id || '';
            addLocalNotice('分叉', `已分叉到 ${newId}`);
            loadSessions();
            if (newId) {
              selectSession(newId);
            }
          }

          // session.send / session.retry 响应 → 如果 model.completed 还没来，用 response payload 兜底显示
          if (msg.success && (respCmd === 'session.send' || respCmd === 'session.retry') && msg.payload?.run_id && isCurrentSession(respSessionId)) {
            currentRunId.value = msg.payload.run_id;
            currentRunStatus.value = msg.payload.status || currentRunStatus.value;
            if (respCmd === 'session.retry') {
              addLocalNotice('重试', `已完成重试: ${msg.payload.run_id}`);
            }
          }
          if (msg.success && msg.payload?.status === 'cancelled' && isCurrentSession(respSessionId)) {
            currentRunStatus.value = 'cancelled';
            if (msg.payload?.content) {
              appendAssistantMessage(
                msg.payload.content,
                msg.payload.partial_preserved ? '已中断，已保留中断前输出' : '已中断'
              );
            }
            streamingText.value = '';
            streamingThinking.value = '';
            isStreaming.value = false;
            scrollToChat();
          } else if (msg.success && msg.payload?.content && typeof msg.payload.content === 'string' && isCurrentSession(respSessionId)) {
            // 只有当最后一条不是 assistant 消息时才兜底推送（避免和 model.completed 或 flushStreaming 重复）
            const lastMsg = chatMessages.value[chatMessages.value.length - 1];
            if (!lastMsg || lastMsg.role !== 'assistant') {
              chatMessages.value.push({ role: 'assistant', text: msg.payload.content, meta: now() });
              scrollToChat();
            }
          }

          // session.messages 响应 → 根据请求来源回填聊天历史或全量消息面板
          if (msg.success && msg.payload?.messages && Array.isArray(msg.payload.messages) && isCurrentSession(respSessionId)) {
            const msgs = msg.payload.messages;
            if (respCmd === 'session.messages.full') {
              fullMessagesTotal.value = msg.payload.total || 0;
              fullMessagesHasMore.value = (msg.payload.page + 1) < (msg.payload.pages || 0);
              const mapped = msgs.map(m => ({ ...m, _new: true, _expanded: false }));
              if ((msg.payload.page || 0) === 0) fullMessages.value = mapped;
              else fullMessages.value.push(...mapped);
              fullMessagesPage.value = (msg.payload.page || 0) + 1;
              fullMessagesLoading.value = false;
              setTimeout(() => fullMessages.value.forEach(m => m._new = false), 800);
            } else if (msgs.length > 0) {
              const mappedMessages = mapProtocolMessagesToChatMessages(msgs);
              chatMessages.value = mappedMessages;
              scrollToChat();
              const loadedCount = msgs.length;
              const totalCount = msg.payload.total || msgs.length;
              const renderedCount = chatMessages.value.length;
              addLocalNotice(
                '会话',
                renderedCount === msgs.length
                  ? (totalCount > loadedCount
                      ? `已加载 ${loadedCount} 条历史消息（会话共 ${totalCount} 条）`
                      : `已加载 ${loadedCount} 条历史消息`)
                  : `已加载 ${loadedCount} 条历史消息，当前显示 ${renderedCount} 条`
              );
            }
          }
          const pendingWaiter = pendingCommandWaiters.get(msg.request_id);
          if (pendingWaiter) {
            pendingCommandWaiters.delete(msg.request_id);
            if (msg.success) {
              pendingWaiter.resolve(msg);
            } else {
              pendingWaiter.reject(new Error(msg.payload?.error || '未知错误'));
            }
          }
          pendingCommands.delete(msg.request_id);
          return;
        }

        // Stream 是 Core 主动推送的运行时流，也归入事件流；命令响应不进入事件流
        if (msg.type === 'stream') {
          addEvent('stream', msg.session_id, msg.data || {}, msg.run_id, msg);
          return;
        }
      };
    }

    function disconnect() {
      autoReconnect.value = false;
      if (reconnectTimer.value) { clearTimeout(reconnectTimer.value); reconnectTimer.value = null; }
      if (ws) ws.close();
    }

    function scheduleReconnect() {
      if (reconnectTimer.value) return;
      reconnectTimer.value = setTimeout(() => {
        reconnectTimer.value = null;
        if (!connected.value && autoReconnect.value) {
          addLocalNotice('WS', '自动重连...');
          connect();
        }
      }, 3000);
    }

    function loadSessionHistory() {
      if (!ws || !sessionId.value) return;
      // 1. 拉取消息历史
      const rid = nextReqId();
      const fullMsg = {
        command: 'session.messages',
        request_id: rid,
        session_id: sessionId.value,
        payload: { page: 0, limit: 50 }
      };
      const out = JSON.stringify(fullMsg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function loadSessions() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'session.list',
        request_id: rid,
        session_id: '',
        payload: { page: 0, limit: 100 },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function loadArchivedSessions() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'session.list',
        request_id: rid,
        session_id: '',
        payload: { page: 0, limit: 100, status: 'archived' },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function toggleArchivedSessions() {
      showArchivedSessions.value = !showArchivedSessions.value;
      if (showArchivedSessions.value) loadArchivedSessions();
    }

    function removeSessionFromVisibleLists(id) {
      sessions.value = sessions.value.filter(item => item.session_id !== id);
      archivedSessions.value = archivedSessions.value.filter(item => item.session_id !== id);
      selectedSessionId.value = sessionId.value;
    }

    function switchToFirstAvailableSession() {
      if (sessions.value.length > 0) {
        selectSession(sessions.value[0].session_id);
        return;
      }
      const newId = 'session_' + Math.random().toString(36).substring(2, 8);
      sessionId.value = newId;
      selectedSessionId.value = newId;
      resetSessionViewState();
      sessions.value = [{ session_id: newId, title: '新会话', message_count: 0 }];
    }

    function resetSessionViewState() {
      chatMessages.value = [];
      streamingText.value = '';
      isStreaming.value = false;
      currentRunId.value = '';
      currentRunStatus.value = 'idle';
      events.value = [];
      rawMessages.value = [];
      toolExecLog.value = [];
      fullMessages.value = [];
      fullMessagesPage.value = 0;
      fullMessagesTotal.value = 0;
      fullMessagesHasMore.value = true;
      fullMessagesLoading.value = false;
      runtimeSessions.value = [];
      runtimeRuns.value = [];
      runtimeRuntimeStatus.value = '';
      runtimeLoading.value = false;
      latestToolChainReport.value = null;
      latestTraceDetails.value = [];
    }

    function createNewSession() {
      const newId = 'session_' + Math.random().toString(36).substring(2, 8);
      sessions.value.unshift({ session_id: newId, title: '新会话', message_count: 0 });
      selectSession(newId);
    }

    function closeSession(id) {
      if (!ws || !id) return;
      if (runtimeSessions.value.includes(id)) {
        addLocalNotice('会话', `session ${id} 当前仍有运行中的任务，暂不允许关闭`);
        return;
      }
      sendCommand('session.close', {}, id);
    }

    function archiveSession(id) {
      if (!ws || !id) return;
      if (runtimeSessions.value.includes(id)) {
        addLocalNotice('会话', `session ${id} 当前仍有运行中的任务，暂不允许归档`);
        return;
      }
      const target = sessions.value.find(s => s.session_id === id) || archivedSessions.value.find(s => s.session_id === id);
      const title = target?.title || id;
      if (!window.confirm(`确认归档会话 "${title}" 吗？\n\n归档只会从默认列表隐藏，历史文件会完整保留。`)) {
        return;
      }
      sendCommand('session.archive', {}, id);
    }

    function unarchiveSession(id) {
      if (!ws || !id) return;
      sendCommand('session.unarchive', {}, id);
    }

    function deleteSession(id) {
      if (!ws || !id) return;
      if (runtimeSessions.value.includes(id)) {
        addLocalNotice('会话', `session ${id} 当前仍有运行中的任务，暂不允许永久删除`);
        return;
      }
      const target = sessions.value.find(s => s.session_id === id) || archivedSessions.value.find(s => s.session_id === id);
      const title = target?.title || id;
      if (!window.confirm(`确认永久删除会话 "${title}" 吗？\n\n这会删除 .aicore/sessions 下的完整历史目录，不能恢复。`)) {
        return;
      }
      sendCommand('session.delete', { permanent: true }, id);
    }

    function forkSession(srcId) {
      if (!ws || !srcId) return;
      const target = sessions.value.find(s => s.session_id === srcId);
      const title = target?.title || srcId;
      const dstId = prompt(`分叉 session "${title}"\n\n请输入新 session ID（留空自动生成）：`);
      if (dstId === null) return; // 用户取消
      const newId = dstId.trim() || `sess_fork_${Date.now()}`;
      if (sessions.value.some(s => s.session_id === newId)) {
        addLocalNotice('分叉', `session "${newId}" 已存在，请换一个 ID`);
        return;
      }
      sendCommand('session.fork', { source_session_id: srcId, new_session_id: newId });
      addLocalNotice('分叉', `正在从 ${title} 分叉到 ${newId}...`);
    }

    function selectSession(id) {
      if (!id || id === sessionId.value) return;
      sessionId.value = id;
      resetSessionViewState();
      loadSessionHistory();
      loadProvider();
      loadSystemPrompt();
      loadTools();
      loadContextPreview();
      loadRuntimeSessions();
      addLocalNotice('会话', `已切换并加载 session: ${id}`);
    }

    function sendCommand(command, payload = {}, sid = sessionId.value, options = {}) {
      if (!ws) return '';
      const rid = nextReqId();
      const msg = { command, request_id: rid, session_id: sid, payload };
      const out = JSON.stringify(msg);
      ws.send(out);
      if (!options.silentRaw) addRawMessage('out', out);
      if (options.silentRaw) {
        pendingCommands.set(rid, {
          command,
          sessionId: sid || '',
          silentRaw: true,
        });
      }
      return rid;
    }

    function sendCommandAwait(command, payload = {}, sid = sessionId.value, options = {}) {
      if (!ws) {
        return Promise.reject(new Error('WS 未连接，无法执行命令'));
      }
      const timeoutMs = options.timeoutMs ?? 120000;
      const rid = sendCommand(command, payload, sid);
      return new Promise((resolve, reject) => {
        const timer = timeoutMs > 0 ? setTimeout(() => {
          pendingCommandWaiters.delete(rid);
          pendingCommands.delete(rid);
          reject(new Error(`${command} 等待响应超时 (${timeoutMs}ms)`));
        }, timeoutMs) : null;
        pendingCommandWaiters.set(rid, {
          resolve: (msg) => {
            if (timer) clearTimeout(timer);
            resolve(msg);
          },
          reject: (err) => {
            if (timer) clearTimeout(timer);
            reject(err instanceof Error ? err : new Error(String(err || '未知错误')));
          },
        });
      });
    }

    async function registerToolForSession(tool, sid) {
      if (!tool || !sid) return;
      await sendCommandAwait('tool.register', {
        tool_name: tool.name,
        description: tool.description || '',
        schema: tool.schema || { type: 'object', properties: {} },
        client_id: 'web_debug_agent',
        timeout_ms: 0,
        tags: tool.tags || [],
      }, sid);
    }

    async function runNestedAgent(message) {
      if (!isAgentToolConfigured()) {
        return {
          ok: false,
          result: 'agent 工具尚未配置完成，请先在 script.js 中补齐 AGENT_TOOL_PROVIDER_TEMPLATE 与 AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE 占位符。',
        };
      }

      const agentSessionId = createEphemeralSessionId();
      try {
        // system_prompt.set 会确保 session 先被创建，随后再写 provider 覆盖。
        await sendCommandAwait('system_prompt.set', {
          system_prompt: AGENT_TOOL_SYSTEM_PROMPT_TEMPLATE,
        }, agentSessionId);

        await sendCommandAwait('provider.update', {
          ...AGENT_TOOL_PROVIDER_TEMPLATE,
        }, agentSessionId);

        for (const tool of getAgentDelegatedTools()) {
          await registerToolForSession(tool, agentSessionId);
        }

        const response = await sendCommandAwait('session.send', {
          message,
          max_repeated_tool_calls: 10,
        }, agentSessionId, { timeoutMs: 0 });
        const content = response?.payload?.content;
        return {
          ok: true,
          result: typeof content === 'string' && content.trim()
            ? content
            : JSON.stringify(response?.payload || {}, null, 2),
        };
      } catch (error) {
        return {
          ok: false,
          result: `agent 子会话执行失败: ${error?.message || error}`,
        };
      } finally {
        try {
          await sendCommandAwait('session.delete', {}, agentSessionId, { timeoutMs: 10000 });
        } catch (cleanupError) {
          addLocalNotice('agent', `子会话清理失败: ${cleanupError?.message || cleanupError}`);
        }
        loadSessions();
      }
    }

    function loadFullMessages(reset = false) {
      if (!ws || !sessionId.value || fullMessagesLoading.value) return;
      if (reset) {
        fullMessages.value = [];
        fullMessagesPage.value = 0;
        fullMessagesTotal.value = 0;
        fullMessagesHasMore.value = true;
      }
      if (!fullMessagesHasMore.value && !reset) return;
      fullMessagesLoading.value = true;
      const rid = nextReqId();
      const msg = {
        command: 'session.messages',
        request_id: rid,
        session_id: sessionId.value,
        payload: { page: fullMessagesPage.value, limit: 200 }
      };
      pendingCommands.set(rid, 'session.messages.full');
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function onFullMessagesScroll() {
      const el = fullMessagesBody.value;
      if (!el || fullMessagesLoading.value || !fullMessagesHasMore.value) return;
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 80) {
        loadFullMessages(false);
      }
    }

    function roleLabel(role) {
      return ({ user: '用户', assistant: '助手', system: '系统', tool: '工具' })[role] || role || '未知';
    }

    function roleColor(role) {
      return ({ user: 'var(--green)', assistant: 'var(--cyan)', system: 'var(--orange)', tool: 'var(--pink)' })[role] || 'var(--text2)';
    }

    function formatMessageTime(value) {
      if (!value) return '';
      try { return new Date(value).toLocaleTimeString('zh-CN', {hour12:false}); } catch { return value; }
    }

    function extractHistoryMessageText(message) {
      if (!message) return '';
      if (typeof message.text === 'string' && message.text) return message.text;
      if (Array.isArray(message.content)) {
        return message.content.map(block => {
          if (!block || typeof block !== 'object') return '';
          if (block.type === 'text') return block.text || '';
          if (block.type === 'tool_result') return block.content || '';
          return '';
        }).join('').trim();
      }
      return '';
    }

    function contextCommandName(cmd) {
      const map = {
        'context.preview': '上下文预览',
        'context.trim.set': '设置裁剪策略',
        'context.exclude': '排除上下文消息',
        'context.seed.add': '新增 Seed',
        'context.seed.set': '覆盖写入 Seed',
        'context.seed.delete': '删除 Seed',
        'context.seed.clear': '清空 Seed',
      };
      return map[cmd] || cmd;
    }

    function contextModeLabel(mode) {
      const map = { full: '全量上下文', sliding: '滑动/裁剪上下文', compacted: '压缩上下文' };
      return map[mode] || mode || '未知';
    }

    function seedKindLabel(kind) {
      const map = { system_memory: '系统记忆', compaction_summary: '压缩摘要', user_preference: '用户偏好', world_state: '世界状态', agent_state: 'Agent状态' };
      return map[kind] || kind || '未知';
    }

    function loadContextPreview() {
      sendCommand('context.preview');
    }

    function setTrimNone() {
      sendCommand('context.trim.set', { mode: 'none' });
    }

    function setContextTrim() {
      const mode = contextTrimMode.value;
      if (mode === 'none') {
        sendCommand('context.trim.set', { mode: 'none' });
        return;
      }
      if (mode === 'keep_recent_messages') {
        const keep_messages = Number(contextKeepMessages.value || 0);
        if (keep_messages <= 0) return;
        sendCommand('context.trim.set', { mode, keep_messages });
        return;
      }
      if (mode === 'include_after') {
        const message_id = contextIncludeAfter.value.trim();
        if (!message_id) return;
        sendCommand('context.trim.set', { mode, message_id });
        return;
      }
      if (mode === 'checkpoint') {
        const trigger_max_context_messages = Number(contextTriggerMaxMessages.value || 0);
        const retain_recent_turns = Number(contextRetainTurns.value || 0);
        if (trigger_max_context_messages <= 0 || retain_recent_turns <= 0) return;
        sendCommand('context.trim.set', { mode, trigger_max_context_messages, retain_recent_turns });
      }
    }

    function excludeSingleMessage(messageId) {
      sendCommand('context.exclude', { start_message_id: messageId, end_message_id: messageId });
    }

    async function copyMessageId(messageId) {
      if (!messageId) return;
      try {
        await navigator.clipboard.writeText(messageId);
        contextStatus.value = `已复制消息 ID：${messageId}`;
      } catch (e) {
        contextIncludeAfter.value = messageId;
        contextStatus.value = '浏览器禁止直接复制，已填入起点输入框';
      }
    }

    function saveContextSeed() {
      const command = contextSeedMode.value === 'set' ? 'context.seed.set' : 'context.seed.add';
      sendCommand(command, {
        kind: contextSeedKind.value,
        content: contextSeedContent.value,
        enabled: true,
        priority: 0,
      });
    }

    function deleteContextSeed(seedId) {
      if (!seedId) return;
      if (!confirm(`确认删除 Seed：${seedId}？`)) return;
      sendCommand('context.seed.delete', { seed_id: seedId });
    }

    function clearContextSeedsByKind() {
      if (!confirm(`确认清空 ${seedKindLabel(contextSeedKind.value)} 类型的所有 Seed？`)) return;
      sendCommand('context.seed.clear', { kind: contextSeedKind.value });
    }

    function clearAllContextSeeds() {
      if (!confirm('确认清空当前 Session 的全部 Seed？')) return;
      sendCommand('context.seed.clear', {});
    }

    function contextMessageText(m) {
      return (m.content || []).map(c => {
        if (c.type === 'text') return c.text || '';
        if (c.type === 'tool_use') return `[tool_use:${c.name}] ${JSON.stringify(c.input || {})}`;
        if (c.type === 'tool_result') return `[tool_result:${c.tool_use_id}] ${c.content || ''}`;
        return JSON.stringify(c);
      }).join('\n');
    }

    function loadTools() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'tool.list',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    const lastRuntimeRefreshAt = { value: 0 };

    function loadRuntimeSessions(options = {}) {
      if (!ws) return;
      const nowTs = Date.now();
      if (!options.force && nowTs - lastRuntimeRefreshAt.value < 800) return;
      lastRuntimeRefreshAt.value = nowTs;
      runtimeLoading.value = true;
      sendCommand('runtime.sessions', {}, '', { silentRaw: options.silentRaw !== false });
    }

    function loadSystemPrompt() {
      if (!ws) return;
      _systemPromptAutoApplied = false;
      const rid = nextReqId();
      const msg = {
        command: 'system_prompt.get',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function saveSystemPrompt() {
      if (!ws) return;
      const rid = nextReqId();
      const msg = {
        command: 'system_prompt.set',
        request_id: rid,
        session_id: sessionId.value,
        payload: { system_prompt: systemPrompt.value || '' },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
      systemPromptStatus.value = '已发送保存请求...';
      setTimeout(() => systemPromptStatus.value = '', 3000);
    }

    function sendChat() {
      const text = chatInput.value.trim();
      const images = pendingImages.value;
      const audio = pendingAudio.value;
      if (!ws || (!text && images.length === 0 && audio.length === 0)) return;

      // 构建聊天区域显示
      const meta = now();
      const msgEntry = { role: 'user', text, meta };
      if (images.length > 0) msgEntry.images = images.map(img => ({ name: img.name, dataUrl: img.dataUrl }));
      if (audio.length > 0) msgEntry.audioFiles = audio.map(a => ({ name: a.name, dataUrl: a.dataUrl, format: a.format, duration: a.duration }));
      chatMessages.value.push(msgEntry);
      chatInput.value = '';
      currentRunId.value = '';
      currentRunStatus.value = 'pending';
      streamingText.value = '';
      streamingThinking.value = '';
      isStreaming.value = false;
      scrollToChat();

      const rid = nextReqId();
      const payload = { message: text };
      if (images.length > 0) payload.images = images.map(img => img.base64);
      if (audio.length > 0) payload.audio = audio.map(a => ({ data: a.base64, format: a.format }));
      const fullMsg = {
        command: 'session.send',
        request_id: rid,
        session_id: sessionId.value,
        payload,
      };
      const out = JSON.stringify(fullMsg);
      ws.send(out);
      addRawMessage('out', out);
      pendingImages.value = [];
      pendingAudio.value = [];
    }

    function retrySession() {
      if (!ws || !sessionId.value || !canRetrySession.value) return;
      currentRunId.value = '';
      currentRunStatus.value = 'pending';
      streamingText.value = '';
      streamingThinking.value = '';
      isStreaming.value = false;
      sendCommand('session.retry', {}, sessionId.value);
      addLocalNotice('重试', '已发送 session.retry，将基于现有消息历史续跑');
    }

    function insertChatMessage(role) {
      if (!ws || !chatInput.value.trim()) return;
      const text = chatInput.value.trim();
      const rid = nextReqId();
      const fullMsg = {
        command: 'session.message.insert',
        request_id: rid,
        session_id: sessionId.value,
        payload: { role, content: text }
      };
      const out = JSON.stringify(fullMsg);
      ws.send(out);
      addRawMessage('out', out);
      chatMessages.value.push({ role, text, meta: now() + ' [已插入]' });
      chatInput.value = '';
      scrollToChat();
    }

    // ─── 图片管理 ───────────────────────────────
    let _imageIdSeq = 0;

    function addImageFiles(files) {
      for (const file of files) {
        if (!file.type.startsWith('image/')) continue;
        if (file.size > 10 * 1024 * 1024) {
          addLocalNotice('图片', `${file.name} 超过 10MB，已跳过`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = (e) => {
          const dataUrl = e.target.result;
          // data:image/png;base64,xxxxx → 提取纯 base64
          const commaIdx = dataUrl.indexOf(',');
          const header = dataUrl.substring(0, commaIdx);
          const base64 = dataUrl.substring(commaIdx + 1);
          const mediaType = header.split(':')[1]?.split(';')[0] || 'image/png';
          pendingImages.value.push({
            id: ++_imageIdSeq,
            name: file.name,
            dataUrl,
            base64,
            mediaType,
          });
        };
        reader.readAsDataURL(file);
      }
    }

    function removePendingImage(id) {
      pendingImages.value = pendingImages.value.filter(img => img.id !== id);
    }

    function clearPendingImages() {
      pendingImages.value = [];
    }

    function handlePaste(e) {
      const items = e.clipboardData?.items;
      if (!items) return;
      const imageFiles = [];
      for (const item of items) {
        if (item.type.startsWith('image/')) {
          const file = item.getAsFile();
          if (file) imageFiles.push(file);
        }
      }
      if (imageFiles.length > 0) {
        e.preventDefault();
        addImageFiles(imageFiles);
      }
    }

    function handleFileSelect(e) {
      const files = e.target.files;
      if (files && files.length > 0) {
        addImageFiles(files);
      }
      e.target.value = ''; // 重置 input 允许重复选同一文件
    }

    function triggerFileSelect() {
      document.getElementById('image-file-input')?.click();
    }

    // ─── 音频管理 ───────────────────────────────
    let _audioIdSeq = 0;
    const AUDIO_FORMAT_MAP = { 'audio/wav': 'wav', 'audio/x-wav': 'wav', 'audio/wave': 'wav', 'audio/mpeg': 'mp3', 'audio/mp3': 'mp3', 'audio/ogg': 'ogg', 'audio/webm': 'webm', 'audio/mp4': 'mp4', 'audio/m4a': 'mp4', 'audio/aac': 'aac' };

    // AudioBuffer → WAV (PCM 16-bit) 编码器
    function audioBufferToWav(buffer) {
      const numChannels = buffer.numberOfChannels;
      const sampleRate = buffer.sampleRate;
      const format = 1; // PCM
      const bitDepth = 16;
      const bytesPerSample = bitDepth / 8;
      const blockAlign = numChannels * bytesPerSample;
      const dataLength = buffer.length * blockAlign;
      const headerLength = 44;
      const totalLength = headerLength + dataLength;
      const arrayBuffer = new ArrayBuffer(totalLength);
      const view = new DataView(arrayBuffer);

      // 交错合并多声道
      let channels = [];
      for (let i = 0; i < numChannels; i++) channels.push(buffer.getChannelData(i));

      // WAV header
      function writeString(offset, str) { for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i)); }
      writeString(0, 'RIFF');
      view.setUint32(4, totalLength - 8, true);
      writeString(8, 'WAVE');
      writeString(12, 'fmt ');
      view.setUint32(16, 16, true);
      view.setUint16(20, format, true);
      view.setUint16(22, numChannels, true);
      view.setUint32(24, sampleRate, true);
      view.setUint32(28, sampleRate * blockAlign, true);
      view.setUint16(32, blockAlign, true);
      view.setUint16(34, bitDepth, true);
      writeString(36, 'data');
      view.setUint32(40, dataLength, true);

      // PCM samples
      let offset = 44;
      for (let i = 0; i < buffer.length; i++) {
        for (let ch = 0; ch < numChannels; ch++) {
          let sample = Math.max(-1, Math.min(1, channels[ch][i]));
          sample = sample < 0 ? sample * 0x8000 : sample * 0x7FFF;
          view.setInt16(offset, sample, true);
          offset += 2;
        }
      }
      return arrayBuffer;
    }

    function addAudioFiles(files) {
      for (const file of files) {
        if (!file.type.startsWith('audio/')) continue;
        if (file.size > 25 * 1024 * 1024) {
          addLocalNotice('音频', `${file.name} 超过 25MB，已跳过`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = (e) => {
          const dataUrl = e.target.result;
          const commaIdx = dataUrl.indexOf(',');
          const base64 = dataUrl.substring(commaIdx + 1);
          const format = AUDIO_FORMAT_MAP[file.type] || file.name.split('.').pop() || 'wav';
          // 创建一个 audio 元素来获取时长
          const audioEl = new Audio(dataUrl);
          audioEl.addEventListener('loadedmetadata', () => {
            pendingAudio.value.push({
              id: ++_audioIdSeq,
              name: file.name,
              dataUrl,
              base64,
              format,
              duration: audioEl.duration,
            });
          });
          // 如果 metadata 加载失败也添加
          audioEl.addEventListener('error', () => {
            pendingAudio.value.push({
              id: ++_audioIdSeq,
              name: file.name,
              dataUrl,
              base64,
              format,
              duration: 0,
            });
          });
        };
        reader.readAsDataURL(file);
      }
    }

    function removePendingAudio(id) {
      pendingAudio.value = pendingAudio.value.filter(a => a.id !== id);
    }

    function clearPendingAudio() {
      pendingAudio.value = [];
    }

    function triggerAudioFileSelect() {
      document.getElementById('audio-file-input')?.click();
    }

    function handleAudioFileSelect(e) {
      const files = e.target.files;
      if (files && files.length > 0) {
        addAudioFiles(files);
      }
      e.target.value = '';
    }

    async function toggleRecording() {
      if (isRecording.value) {
        // 停止录音
        if (_mediaRecorder && _mediaRecorder.state !== 'inactive') {
          _mediaRecorder.stop();
        }
        return;
      }
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        // 优先选 wav，回退到可用格式
        const mimeType = MediaRecorder.isTypeSupported('audio/wav') ? 'audio/wav'
          : MediaRecorder.isTypeSupported('audio/webm;codecs=opus') ? 'audio/webm;codecs=opus'
          : MediaRecorder.isTypeSupported('audio/webm') ? 'audio/webm'
          : 'audio/ogg';
        const format = AUDIO_FORMAT_MAP[mimeType] || 'webm';
        _mediaRecorder = new MediaRecorder(stream, { mimeType });
        _recordedChunks = [];
        isRecording.value = true;

        _mediaRecorder.ondataavailable = (e) => {
          if (e.data.size > 0) _recordedChunks.push(e.data);
        };

        _mediaRecorder.onstop = async () => {
          isRecording.value = false;
          stream.getTracks().forEach(t => t.stop());
          const blob = new Blob(_recordedChunks, { type: mimeType });
          if (blob.size === 0) return;
          try {
            // 用 Web Audio API 解码，再编码为 WAV
            const audioCtx = new (window.AudioContext || window.webkitAudioContext)();
            const arrayBuf = await blob.arrayBuffer();
            const audioBuffer = await audioCtx.decodeAudioData(arrayBuf);
            const wavBuf = audioBufferToWav(audioBuffer);
            const wavBlob = new Blob([wavBuf], { type: 'audio/wav' });
            const reader = new FileReader();
            reader.onload = (e) => {
              const dataUrl = e.target.result;
              const commaIdx = dataUrl.indexOf(',');
              const base64 = dataUrl.substring(commaIdx + 1);
              pendingAudio.value.push({
                id: ++_audioIdSeq,
                name: `录音_${new Date().toLocaleTimeString('zh-CN', {hour12:false}).replace(/:/g,'')}.wav`,
                dataUrl,
                base64,
                format: 'wav',
                duration: audioBuffer.duration,
              });
            };
            reader.readAsDataURL(wavBlob);
          } catch (err) {
            addLocalNotice('录音', '音频转换失败: ' + err.message);
          }
        };

        _mediaRecorder.start();
      } catch (err) {
        addLocalNotice('录音', '无法访问麦克风: ' + err.message);
      }
    }

    function formatDuration(seconds) {
      if (!seconds || seconds <= 0) return '';
      const m = Math.floor(seconds / 60);
      const s = Math.floor(seconds % 60);
      return `${m}:${String(s).padStart(2, '0')}`;
    }

    function cancelCurrentRun() {
      if (!ws || !currentRunId.value) return;
      currentRunStatus.value = 'cancelling';
      sendCommand('run.cancel', { run_id: currentRunId.value });
    }

    function sendRaw() {
      if (!ws || !rawInput.value.trim()) return;
      const text = rawInput.value.trim();
      let parsed;
      try { parsed = JSON.parse(text); } catch(e) { addLocalNotice('原始命令', 'JSON 解析失败: ' + e.message); return; }
      if (!parsed.request_id) parsed.request_id = nextReqId();
      const out = JSON.stringify(parsed);
      ws.send(out);
      addRawMessage('out', out);
      rawInput.value = '';
    }

    function sendToolResult(call_id, result, is_error, sid = sessionId.value) {
      if (!ws) return;
      const msg = {
        command: 'tool.execute.result',
        request_id: nextReqId(),
        session_id: sid,
        payload: { call_id, result: String(result), is_error: !!is_error },
      };
      const out = JSON.stringify(msg);
      ws.send(out);
      addRawMessage('out', out);
    }

    function flushStreaming() {
      // 将正在流式显示的内容刷入正式消息列表（避免 response 和 event 竞争丢失）
      if (isStreaming.value && streamingText.value) {
        appendAssistantMessage(streamingText.value, now());
        streamingText.value = '';
        isStreaming.value = false;
        scrollToChat();
      }
    }

    // 工具队列管理，用于确保带有阻塞的工具按顺序执行
    const toolCallQueue = [];
    let isProcessingToolCall = false;

    async function processNextToolCall() {
      if (isProcessingToolCall || toolCallQueue.length === 0) return;
      isProcessingToolCall = true;
      const { payload, eventSessionId } = toolCallQueue.shift();
      try {
        await executeToolCall(payload, eventSessionId);
      } finally {
        isProcessingToolCall = false;
        processNextToolCall();
      }
    }

    async function executeToolCall(payload, eventSessionId) {
      const { tool_name, call_id, input } = payload || {};
      const safeInput = input || {};
      const tool = toolRegistry.get(tool_name);
      const startTime = Date.now();
      const belongsToCurrentSession = isCurrentSession(eventSessionId);
      const activeRunId = payload?.run_id || currentRunId.value;

      // 先结束流式，避免工具执行日志插入时和模型增量混在一起
      if (belongsToCurrentSession) {
        flushStreaming();
      }

      let pendingTc = null;
      if (belongsToCurrentSession) {
        const lastMsg = ensureActiveToolChainMessage(activeRunId);

        pendingTc = {
          id: call_id,
          name: tool_name,
          input: safeInput,
          result: null,
          isError: false,
          status: 'pending'
        };
        lastMsg.toolCalls.push(pendingTc);
        scrollToChat();
      }

      let execResult;
      if (!tool) {
        execResult = { ok: false, result: `tool '${tool_name}' not found in client registry` };
      } else {
        try {
          const value = await tool.execute(safeInput, {
            toolName: tool_name,
            callId: call_id,
            sessionId: eventSessionId || sessionId.value,
            connectionId: connectionId.value,
          });
          execResult = normalizeToolExecutionResult(value);
        } catch(e) {
          execResult = { ok: false, result: '执行异常: ' + e.message };
        }
      }

      const elapsed = Date.now() - startTime;
      if (belongsToCurrentSession && pendingTc) {
        pendingTc.result = execResult.result;
        pendingTc.isError = !execResult.ok;
        pendingTc.status = execResult.ok ? 'done' : 'error';
        pendingTc.elapsed = elapsed;

        toolExecLog.value.push({
          time: now(), tool: tool_name, input: JSON.stringify(safeInput),
          result: execResult.result, ok: execResult.ok, elapsed,
        });
        if (toolExecLog.value.length > 100) toolExecLog.value.splice(0, toolExecLog.value.length - 100);
        scrollToChat();
      }

      sendToolResult(call_id, execResult.result, !execResult.ok, eventSessionId || sessionId.value);
    }

    async function handleToolCallRequest(payload, eventSessionId) {
      if (payload?.tool_name === 'agent') {
        void executeToolCall(payload, eventSessionId);
        return;
      }
      toolCallQueue.push({ payload, eventSessionId });
      processNextToolCall();
    }

    function handleEvent(type, payload, envelope = {}) {
      const eventSessionId = envelope.session_id || payload?.session_id || '';
      const eventRunId = envelope.run_id || payload?.run_id || '';
      const belongsToCurrentSession = isCurrentSession(eventSessionId);
      if (belongsToCurrentSession && type === 'run.started' && eventRunId && (!currentRunId.value || currentRunStatus.value === 'pending')) {
        currentRunId.value = eventRunId;
      }
      const belongsToTrackedRun = !eventRunId || !currentRunId.value || eventRunId === currentRunId.value;
      // tool.call.request → 统一工具调用入口；业务端只需要注册工具 execute(input, ctx)
      if (type === 'tool.call.request') {
        handleToolCallRequest(payload, eventSessionId);
        return;
      }
      if (!belongsToCurrentSession) {
        if (type === 'run.started' || type === 'run.completed' || type === 'run.cancelled' || type === 'run.failed') {
          loadRuntimeSessions();
        }
        return;
      }
      if (type === 'run.started') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'running';
        streamingThinking.value = '';
        return;
      }
      if (type === 'tool_chain.diagnosed') {
        latestToolChainReport.value = payload.report || null;
        return;
      }
      // model.delta → 实时累积流式文本
      if (type === 'model.delta' && payload.delta) {
        if (!belongsToTrackedRun) return;
        if (payload.event_type === 'thinking') {
          streamingThinking.value += payload.delta;
        } else {
          currentRunStatus.value = 'streaming';
          isStreaming.value = true;
          streamingText.value += payload.delta;
        }
        scrollToChat();
      }
      // model.completed → 最终确认（不用 setTimeout，避免与 response 到达产生竞争）
      if (type === 'model.completed' && payload.content) {
        if (!belongsToTrackedRun) return;
        // 用最终 content 确保完整
        if (payload.content) {
          streamingText.value = payload.content;
        }
        isStreaming.value = false;
        // 立即转为正式消息
        appendAssistantMessage(streamingText.value || payload.content, now());
        streamingText.value = '';
        streamingThinking.value = '';
        currentRunStatus.value = 'completed';
        scrollToChat();
        return;
      }
      if (type === 'run.completed') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'completed';
        loadRuntimeSessions();
        scrollToChat();
        return;
      }
      if (type === 'run.cancelled') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        const partialContent = payload.partial_content || streamingText.value || '';
        if (partialContent) {
          appendAssistantMessage(
            partialContent,
            payload.preserved ? '已中断，已保留中断前输出' : '已中断'
          );
        }
        streamingText.value = '';
        streamingThinking.value = '';
        isStreaming.value = false;
        currentRunStatus.value = 'cancelled';
        loadRuntimeSessions();
        scrollToChat();
        return;
      }
      if (type === 'run.failed') {
        if (!belongsToTrackedRun) {
          loadRuntimeSessions();
          return;
        }
        currentRunStatus.value = 'failed';
        loadRuntimeSessions();
        return;
      }
      // tool.registered → 更新工具列表
      if (type === 'tool.registered') {
        // tools will be refreshed from tool.register commands
      }
      // model.delta → 可以选择实时显示流式内容
    }

    // ─── 事件类型中文标签 ────────────────────────────────
    function eventLabel(type) {
      const map = {
        'model.delta': '模型流式增量',
        'model.completed': '模型文本已收完',
        'tool_chain.diagnosed': '上下文工具链检查',
        'session.created': '会话创建',
        'session.closed': '会话关闭',
        'session.archived': '会话归档',
        'session.unarchived': '会话恢复',
        'session.deleted': '会话永久删除',
        'run.started': '推理任务开始',
        'run.cancelled': '推理任务中断',
        'run.completed': '推理任务结束',
        'tool.call.request': '工具执行请求',
        'tool.call.result': 'Core 已确认工具结果',
        'tool.call.error': '工具调用失败',
        'tool.registered': '工具注册',
        'context.threshold.reached': '上下文阈值',
        'context.updated': '上下文更新',
        'context.seed.added': 'Seed 已新增',
        'context.seed.updated': 'Seed 已覆盖',
        'context.seed.deleted': 'Seed 已删除',
        'context.seed.cleared': 'Seed 已清空',
        'prompt.attached': 'Prompt 附加',
        'run.failed': '推理任务失败',
        'stream': '流数据',
      };
      return map[type] || type;
    }

    function summarizeEvent(type, p) {
      p = p || {};
      const text = (v, n = 60) => String(v ?? '').slice(0, n);
      const tag = (v, cls) => `<span class="raw-tag${cls ? ' '+cls : ''}">${String(v ?? '')}</span>`;
      switch(type) {
        case 'model.delta': return `${p.event_type === 'thinking' ? '思维' : '文本'}增量 ${tag(JSON.stringify(p.delta || '').slice(0, 48))}`;
        case 'model.completed': return `AI 最终文本 ${tag(text(p.content, 80))}`;
        case 'tool_chain.diagnosed':
          return `${tag(toolChainStatusText(p.report))} 完整 ${tag((p.report?.complete_tool_call_ids || []).length)} 丢弃 ${tag(toolChainIssueCount(p.report))}`;
        case 'tool.call.request': return `${tag(p.tool_name)} 输入 ${tag(JSON.stringify(p.input || {}).slice(0, 64))}`;
        case 'tool.call.result': return `${tag(p.tool_name)} ${p.is_error ? tag('失败','err') : tag('成功')} 结果 ${text(p.result, 56)}`;
        case 'tool.call.error': return `${tag(p.tool_name)} ${tag(text(p.error, 64), 'err')}`;
        case 'tool.registered': return `${tag(p.tool_name)} 客户端 ${tag(p.client_id || '—')}`;
        case 'session.created': return `标题 ${tag(p.title || (p.auto_created ? '自动创建' : '—'))}`;
        case 'session.closed': return p.reason ? `原因 ${tag(p.reason)}` : '会话关闭';
        case 'run.started': return `任务已进入运行队列：供应商 ${tag(p.provider || '—')} 模型 ${tag(p.model || '—')}`;
        case 'run.cancelled': return `${p.preserved ? tag('已保留部分输出') : tag('未保留输出')} 耗时 ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}`;
        case 'run.completed': return `本轮任务结束：Token ${tag(p.total_tokens || '—')} 耗时 ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}`;
        case 'run.failed': return tag(JSON.stringify(p).slice(0, 90), 'err');
        case 'context.threshold.reached': return `使用率 ${tag((p.usage_percent ?? '—') + '%')} Token ${tag(p.estimated_tokens || '—')}`;
        case 'context.updated': return `动作 ${tag(contextCommandName(p.action || 'context.updated'))} Mode ${tag(contextModeLabel(p.context?.mode || '—'))}`;
        case 'context.seed.added': return `Seed ${tag(p.seed?.seed_id || '—')} ${tag(p.seed?.kind || '')}`;
        case 'context.seed.updated': return `Seed ${tag(p.seed?.seed_id || '—')} ${tag(p.seed?.kind || '')}`;
        case 'context.seed.deleted': return `Seed ${tag(p.seed_id || '—')}`;
        case 'context.seed.cleared': return `类型 ${tag(p.kind || '全部')} 删除 ${tag(p.removed_count || 0)} 条`;
        case 'prompt.attached': return `Prompt ${tag(p.prompt_name || p.name || '—')}`;
        case 'stream': return `流数据 ${tag(JSON.stringify(p).slice(0, 80))}`;
        default: return JSON.stringify(p).slice(0, 100);
      }
    }

    function eventCategory(type) {
      if (type === 'run.failed') return 'error';
      if (type.startsWith('model.')) return 'model';
      if (type.startsWith('run.')) return 'run';
      if (type.startsWith('tool_chain.')) return 'tool';
      if (type.startsWith('tool.')) return 'tool';
      if (type.startsWith('session.')) return 'session';
      if (type.startsWith('context.')) return 'context';
      return 'other';
    }

    function addEvent(type, session, payload, runId, rawEnvelope) {
      const data = JSON.stringify(rawEnvelope || { type: 'event', event_type: type, session_id: session, run_id: runId, payload }, null, 2);
      const category = eventCategory(type);
      const shouldMerge = type === 'model.delta';
      const last = events.value[events.value.length - 1];
      if (
        shouldMerge &&
        last &&
        last._merged &&
        last.type === type &&
        last.runId === runId &&
        last.session === session &&
        last._mergeKey === `${type}:${payload?.event_type || 'text'}`
      ) {
        last.count++;
        last.children.push({
          time: now(),
          payload,
          data,
          brief: summarizeEvent(type, payload),
        });
        last.data = data;
        last.time = now();
        last.brief = `${payload?.event_type === 'thinking' ? '思维' : '文本'}增量，已聚合 ${last.count} 条`;
        nextTick(() => { if (streamBody.value) streamBody.value.scrollTop = streamBody.value.scrollHeight; });
        return;
      }

      events.value.push({
        type,
        category,
        session,
        brief: summarizeEvent(type, payload),
        runId,
        data,
        time: now(),
        label: eventLabel(type),
        _new: true,
        _expanded: false,
        _merged: shouldMerge,
        _mergeKey: `${type}:${payload?.event_type || 'text'}`,
        count: 1,
        children: shouldMerge ? [{
          time: now(),
          payload,
          data,
          brief: summarizeEvent(type, payload),
        }] : [],
      });
      if (events.value.length > 500) events.value.splice(0, events.value.length - 500);
      nextTick(() => { if (streamBody.value) streamBody.value.scrollTop = streamBody.value.scrollHeight; });
      setTimeout(() => { const e = events.value[events.value.length-1]; if(e) e._new = false; }, 800);
    }

    function addLocalNotice(title, message) {
      chatMessages.value.push({ role: 'system', text: `${title}: ${message || ''}`, meta: now() });
      scrollToChat();
    }

    // ─── 原始消息中文解析 ──────────────────────────────
    function summarizeRaw(dir, raw) {
      let msg;
      try { msg = JSON.parse(raw); } catch { return { label: '无法解析', brief: raw.slice(0, 60) }; }

      const tag = (v, cls) => `<span class="raw-tag${cls ? ' '+cls : ''}">${v}</span>`;

      // ─── 收到的消息 ───
      if (dir === 'in') {
        // Response
        if (msg.type === 'response') {
          if (getPendingMeta(msg.request_id).silentRaw) {
            return { label: '', brief: '', hidden: true };
          }
          if (msg.request_id === 'hello') {
            return { label: '连接确认', brief: `连接ID ${tag((msg.payload?.connection_id||'').slice(0,8))}` };
          }
          const status = msg.success ? '成功' : '失败';
          const cls = msg.success ? '' : 'err';
          const p = msg.payload || {};
          // 根据 request_id 查找对应的命令类型
          const cmd = getPendingMeta(msg.request_id).command || '';
          const cmdLabels = {
            'provider.update': '更新供应商响应',
            'provider.get': '获取供应商响应',
            'session.send': '发送消息命令完成',
            'session.retry': '重试会话响应',
            'session.messages': '消息历史响应',
            'session.get': '获取会话响应',
            'session.info': '会话详情响应',
            'session.close': '关闭会话响应',
            'session.archive': '归档会话响应',
            'session.unarchive': '恢复归档响应',
            'session.delete': '永久删除会话响应',
            'session.fork': '分叉会话响应',
            'session.list': '会话列表响应',
            'tool.register': '注册工具响应',
            'tool.unregister': '注销工具响应',
            'system_prompt.get': '获取系统提示词响应',
            'system_prompt.set': '设置系统提示词响应',
            'tool.list': '工具列表响应',
            'tool.get': '工具详情响应',
            'runtime.sessions': '运行中会话响应',
            'system.stats': '系统统计响应',
            'context.preview': '上下文预览响应',
            'context.trim.set': '设置裁剪策略响应',
            'context.exclude': '排除上下文响应',
            'context.seed.add': '新增 Seed 响应',
            'context.seed.set': '覆盖 Seed 响应',
            'context.seed.delete': '删除 Seed 响应',
            'context.seed.clear': '清空 Seed 响应',
            'run.cancel': '取消推理响应',
          };
          let respLabel = cmdLabels[cmd] || '命令响应';
            if (cmd === 'session.send') {
              if (!msg.success) respLabel = 'AI 推理失败';
              else if (p.status === 'cancelled') respLabel = 'AI 推理已中断';
              else if (p.status === 'completed') respLabel = 'AI 推理已结束';
              else respLabel = '发送消息命令完成';
            }
          let brief = `${tag(msg.request_id)} ${tag(status, cls)}`;
          // 按命令类型提取关键信息
          if (p.provider) {
            const pr = p.provider;
            brief += ` ${tag((pr.protocol||'')+'/'+(pr.model||''))}`;
            if (pr.api_key) brief += ` key:${tag(pr.api_key.slice(0,8)+'...')}`;
            if (p.is_override !== undefined) brief += ` ${tag(p.is_override?'会话覆盖':'全局默认')}`;
          }
          if (p.messages) brief += ` 加载 ${p.messages.length} 条历史`;
          if (cmd === 'session.send') {
            if (p.run_id) brief += ` run ${tag((p.run_id || '').slice(0, 16))}`;
            if (p.status) brief += ` 状态 ${tag(p.status, p.status === 'cancelled' ? 'err' : '')}`;
            if (typeof p.traces === 'number') brief += ` 模型调用 ${tag(p.traces)} 次`;
          }
          if (cmd === 'runtime.sessions') brief += ` session ${tag(p.running_session_count ?? 0)} run ${tag(p.running_run_count ?? 0)}`;
          if (p.active_context) {
            brief += ` ${tag(contextModeLabel(p.active_context.mode))}`;
            if (p.counts) brief += ` 可见${tag(p.counts.active_messages || 0)}条 / 全量${tag(p.counts.all_messages || 0)}条`;
          }
          if (Array.isArray(p.seeds) && p.seeds.length) {
            brief += ` Seed${tag(p.seeds.length)}`;
          }
          if (p.seed) brief += ` ${tag(seedKindLabel(p.seed.kind))} ${tag((p.seed.seed_id || '').slice(0,16))}`;
          if (cmd === 'context.trim.set' && p.active_context?.rules?.trim) {
            const trim = p.active_context.rules.trim;
            brief += ` 策略 ${tag(trim.mode || 'none')}`;
            if (trim.keep_messages) brief += ` 保留${tag(trim.keep_messages)}条`;
            if (trim.message_id) brief += ` 起点${tag(trim.message_id.slice(0,16))}`;
            if (trim.trigger_max_context_messages) brief += ` 阈值${tag(trim.trigger_max_context_messages)}条`;
            if (trim.retain_recent_turns) brief += ` 保留${tag(trim.retain_recent_turns)}轮`;
          }
          if (cmd === 'context.exclude' && p.active_context?.rules?.exclude_ranges) brief += ` 已排除 ${tag(p.active_context.rules.exclude_ranges.length)} 段`;
          if (p.content) brief += ` ${tag(JSON.stringify(p.content).slice(0,40))}`;
          if (p.session_id && !p.provider && !p.messages && !p.content && !p.active_context && !p.seed) brief += ` ${tag(p.session_id)}`;
          if (p.error) brief += ` ${tag(p.error.slice(0,40), 'err')}`;
          return { label: respLabel, brief };
        }
        // Event
        if (msg.type === 'event') {
          const et = msg.event_type || 'unknown';
          const p = msg.payload || {};
          const map = {
            'model.delta':              () => ({ label: p.event_type === 'thinking' ? '模型思维（流式）' : '模型输出（流式）', brief: `${p.event_type === 'thinking' ? '思维' : '文本'}增量 ${tag(JSON.stringify(p.delta||'').slice(0,40))}` }),
            'model.completed':          () => ({ label: 'AI 文本已收完', brief: `最终回答 ${tag((p.content||'').slice(0,60))}` }),
            'tool_chain.diagnosed':     () => ({ label: '上下文工具链检查', brief: `${tag(toolChainStatusText(p.report), toolChainIssueCount(p.report) > 0 ? 'err' : '')} 完整闭环 ${tag((p.report?.complete_tool_call_ids||[]).length)} 个，丢弃 ${tag(toolChainIssueCount(p.report))} 个` }),
            'session.created':          () => ({ label: '会话创建', brief: `标题 ${tag(p.title||'—')}` }),
            'session.closed':           () => ({ label: '会话关闭', brief: p.reason ? `原因 ${tag(p.reason)}` : '' }),
            'session.archived':         () => ({ label: '会话归档', brief: '已从默认列表隐藏' }),
            'session.unarchived':       () => ({ label: '会话恢复', brief: '已恢复到普通列表' }),
            'session.deleted':          () => ({ label: '会话永久删除', brief: '持久化目录已删除' }),
            'run.started':              () => ({ label: '推理任务开始', brief: `使用 ${tag(p.provider)} / ${tag(p.model)}` }),
            'run.cancelled':            () => ({ label: '推理任务中断', brief: `${p.preserved ? tag('已保留部分输出') : tag('未保留输出')} ${tag(p.duration_ms ? p.duration_ms + 'ms' : '—')}` }),
            'run.completed':            () => ({ label: '推理任务结束', brief: `本轮耗时 ${tag(p.duration_ms?p.duration_ms+'ms':'—')}` }),
            'tool.call.request':        () => ({ label: '工具执行请求', brief: `${tag(p.tool_name)} 输入 ${JSON.stringify(p.input||{}).slice(0,50)}` }),
            'tool.call.result':         () => ({ label: 'Core 已确认工具结果', brief: `${tag(p.tool_name)} 结果 ${(p.result||'').slice(0,40)} ${p.is_error?tag('错误','err'):tag('已接收')}` }),
            'tool.call.error':          () => ({ label: '工具调用失败', brief: `${tag(p.tool_name)} ${tag((p.error||'').slice(0,40),'err')}` }),
            'tool.registered':          () => ({ label: '工具注册', brief: `${tag(p.tool_name)} 客户端 ${tag(p.client_id)}` }),
            'context.threshold.reached':() => ({ label: '上下文阈值', brief: `使用 ${tag(p.usage_percent+'%')} Token ${tag(p.estimated_tokens)}` }),
            'context.updated':          () => ({ label: '上下文已更新', brief: `${tag(contextCommandName(p.action || 'context.updated'))} ${tag(contextModeLabel(p.context?.mode))}` }),
            'context.seed.added':       () => ({ label: 'Seed 已新增', brief: `${tag(seedKindLabel(p.seed?.kind))} ${tag((p.seed?.seed_id||'').slice(0,16))}` }),
            'context.seed.updated':     () => ({ label: 'Seed 已覆盖', brief: `${tag(seedKindLabel(p.seed?.kind))} ${tag((p.seed?.seed_id||'').slice(0,16))}` }),
            'context.seed.deleted':     () => ({ label: 'Seed 已删除', brief: `${tag((p.seed_id||'').slice(0,16))}` }),
            'context.seed.cleared':     () => ({ label: 'Seed 已清空', brief: `${tag(p.kind || '全部')} 删除 ${tag(p.removed_count || 0)} 条` }),
            'prompt.attached':          () => ({ label: 'Prompt 附加', brief: `${tag(p.prompt_name||p.name||'')}` }),
            'run.failed':               () => ({ label: '推理任务失败', brief: tag(JSON.stringify(p).slice(0,60), 'err') }),
          };
          const factory = map[et];
          if (factory) return factory();
          return { label: et, brief: JSON.stringify(p).slice(0, 60) };
        }
        // Stream
        if (msg.type === 'stream') {
          return { label: '流数据', brief: `事件 ${tag(msg.event)} ${JSON.stringify(msg.data||{}).slice(0,50)}` };
        }
        return { label: msg.type || '未知', brief: JSON.stringify(msg).slice(0, 60) };
      }

      // ─── 发出的消息 ───
      const cmd = msg.command || '';
      const sid = msg.session_id ? tag(msg.session_id) : '';
      const p = msg.payload || {};
      const cmdMap = {
        'session.send':         () => ({ label: '发送消息', brief: `${sid} ${tag(JSON.stringify(p.message||'').slice(0,50))}` }),
        'session.retry':        () => ({ label: '重试会话', brief: `${sid} 基于现有历史续跑` }),
        'session.messages':     () => ({ label: '获取消息历史', brief: `${sid} 页${p.page||0} 每页${p.limit||50}` }),
        'session.get':          () => ({ label: '获取会话', brief: sid }),
        'session.info':         () => ({ label: '会话详情', brief: sid }),
        'session.close':       () => ({ label: '关闭会话', brief: sid }),
        'session.archive':     () => ({ label: '归档会话', brief: sid }),
        'session.unarchive':   () => ({ label: '恢复归档', brief: sid }),
        'session.delete':      () => ({ label: '永久删除会话', brief: sid }),
        'session.list':         () => ({ label: '列出会话', brief: `页${p.page||0} 每页${p.limit||20}` }),
        'provider.update':      () => ({ label: '更新供应商', brief: `${sid} ${tag(p.protocol)} ${tag(p.model)}` }),
        'provider.get':         () => ({ label: '获取供应商', brief: sid }),
        'tool.register':        () => ({ label: '注册工具', brief: `${tag(p.tool_name)} 客户端 ${tag(p.client_id||'')}` }),
        'tool.unregister':      () => ({ label: '注销工具', brief: tag(p.tool_name) }),
        'system_prompt.get':     () => ({ label: '获取系统提示词', brief: sid }),
        'system_prompt.set':     () => ({ label: '设置系统提示词', brief: `${sid} ${tag((p.system_prompt||'').length + ' 字')}` }),
        'tool.execute.result':  () => ({ label: '提交工具结果', brief: `${sid} call_id ${tag((p.call_id||'').slice(0,16))} ${p.is_error?tag('错误','err'):tag('成功')}` }),
        'tool.list':             () => ({ label: '获取工具列表', brief: sid }),
        'tool.get':              () => ({ label: '获取工具详情', brief: tag(p.tool_name) }),
        'runtime.sessions':      () => ({ label: '查询运行中会话', brief: '当前 runtime 状态' }),
        'system.stats':         () => ({ label: '系统统计', brief: '' }),
        'context.preview':      () => ({ label: '上下文预览', brief: sid }),
        'context.seed.add':     () => ({ label: '新增 Seed', brief: `${sid} ${tag(seedKindLabel(p.kind))}` }),
        'context.seed.set':     () => ({ label: '覆盖 Seed', brief: `${sid} ${tag(seedKindLabel(p.kind))}` }),
        'context.seed.delete':  () => ({ label: '删除 Seed', brief: `${sid} ${tag((p.seed_id||'').slice(0,16))}` }),
        'context.seed.clear':   () => ({ label: '清空 Seed', brief: `${sid} ${tag(p.kind || '全部')}` }),
        'run.cancel':           () => ({ label: '取消推理', brief: sid }),
      };
      const factory = cmdMap[cmd];
      if (factory) return factory();
      return { label: cmd || '未知命令', brief: sid || JSON.stringify(p).slice(0, 40) };
    }

    function addRawMessage(dir, data) {
      const { label, brief, hidden } = summarizeRaw(dir, data);
      if (hidden) return;

      // 记录发出的命令类型（用于响应中文标签）
      if (dir === 'out') {
        try {
          const out = JSON.parse(data);
          if (out.command && out.request_id) {
            // 保留已提前标记的特殊命令别名，例如 session.messages.full
            if (!pendingCommands.has(out.request_id)) {
              pendingCommands.set(out.request_id, {
                command: out.command,
                sessionId: out.session_id || '',
              });
            }
            // 清理旧记录（防止内存泄漏）
            if (pendingCommands.size > 200) {
              const keys = [...pendingCommands.keys()];
              keys.slice(0, 100).forEach(k => pendingCommands.delete(k));
            }
          }
        } catch {}
      }

      // 检测是否是连续的 model.delta 事件，如果是则合并到上一条
      if (dir === 'in') {
        let parsed;
        try { parsed = JSON.parse(data); } catch {}
        if (parsed && parsed.type === 'event' && parsed.event_type === 'model.delta') {
          const last = rawMessages.value[rawMessages.value.length - 1];
          if (last && last._merged) {
            // 合并到已有行
            last.count++;
            last.children.push({ data, time: now() });
            last.brief = `已接收 ${last.count} 条流式增量`;
            last.data = data; // 最新的 raw 作为主数据
            last.time = now();
            nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
            return;
          }
          // 创建新的可合并行
          rawMessages.value.push({
            dir, data, time: now(), label, brief,
            _new: true, _expanded: false,
            _merged: true, count: 1,
            children: [{ data, time: now() }],
          });
          if (rawMessages.value.length > 500) rawMessages.value.splice(0, rawMessages.value.length - 500);
          nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
          setTimeout(() => { const m = rawMessages.value[rawMessages.value.length-1]; if(m) m._new = false; }, 800);
          return;
        }
      }

      // 普通消息：直接追加
      rawMessages.value.push({ dir, data, time: now(), label, brief, _new: true, _expanded: false });
      if (rawMessages.value.length > 500) rawMessages.value.splice(0, rawMessages.value.length - 500);
      nextTick(() => { if (rawBody.value) rawBody.value.scrollTop = rawBody.value.scrollHeight; });
      setTimeout(() => { const m = rawMessages.value[rawMessages.value.length-1]; if(m) m._new = false; }, 800);
    }

    function scrollToChat() {
      nextTick(() => { if (chatBody.value) chatBody.value.scrollTop = chatBody.value.scrollHeight; });
    }

    const filteredEvents = computed(() => {
      if (eventFilter.value === 'all') return events.value;
      return events.value.filter(e => e.category === eventFilter.value);
    });

    const filteredRaw = computed(() => {
      if (rawFilter.value === 'all') return rawMessages.value;
      return rawMessages.value.filter(m => m.dir === rawFilter.value);
    });

    function saveProvider() {
      if (!ws) return;
      const rid = nextReqId();
      const payload = { ...providerConfig.value };
      // 去除空值
      if (!payload.api_key) delete payload.api_key;
      ws.send(JSON.stringify({
        command: 'provider.update',
        request_id: rid,
        session_id: sessionId.value,
        payload,
      }));
      addRawMessage('out', JSON.stringify({ command: 'provider.update', request_id: rid, session_id: sessionId.value, payload }));
      providerStatus.value = '已发送配置更新请求...';
      setTimeout(() => providerStatus.value = '', 3000);
    }

    function loadProvider() {
      if (!ws) return;
      const rid = nextReqId();
      ws.send(JSON.stringify({
        command: 'provider.get',
        request_id: rid,
        session_id: sessionId.value,
        payload: {},
      }));
      addRawMessage('out', JSON.stringify({ command: 'provider.get', request_id: rid, session_id: sessionId.value, payload: {} }));
    }

    function applyTemplate(name) {
      const templates = {
        deepseek: { protocol: 'openai', base_url: 'https://api.deepseek.com', model: 'deepseek-chat', max_tokens: 4096, temperature: 0, supports_image: false, supports_audio: false },
        openai:   { protocol: 'openai', base_url: 'https://api.openai.com', model: 'gpt-4o', max_tokens: 4096, temperature: 0, supports_image: true, supports_audio: true },
        claude:   { protocol: 'claude', base_url: 'https://ai.accbot.vip', model: 'claude-sonnet-4-20250514', max_tokens: 4096, temperature: 0, supports_image: true, supports_audio: false },
        ollama:   { protocol: 'openai', base_url: 'http://localhost:11434', model: 'qwen2.5:7b', max_tokens: 4096, temperature: 0, supports_image: false, supports_audio: false },
      };
      const t = templates[name];
      if (t) {
        Object.assign(providerConfig.value, t);
        saveProvider(); // 点击模板后自动保存并应用
      }
    }

    // 页面加载：自动连接（不做 localStorage 持久化，网页仅为调试用）
    onMounted(() => {
      if (wsUrl.value) connect();
      window.addEventListener('beforeunload', () => {
        if (ws) ws.close();
      });
    });

    return {
      wsUrl, connected, connectionId, sessionId,
      wsDropdownOpen, wsCustomMode, wsPresets, wsSelectLabel, selectWsPreset,
      selectedSessionId, sessions, archivedSessions, showArchivedSessions, activeSessionMenu,
      chatInput, chatMessages, pendingImages, pendingAudio, isRecording,
      canRetrySession,
      fullMessages, fullMessagesPage, fullMessagesTotal, fullMessagesHasMore, fullMessagesLoading,
      runtimeSessions, runtimeRuns, runtimeRuntimeStatus, runtimeLoading,
      events, rawMessages, tools, toolExecLog, presetTools,
      toolChainSummary, toolChainStatus,
      rightTab, eventFilter, rawFilter, rawInput,
      chatBody, streamBody, rawBody, fullMessagesBody,
      providerConfig, providerStatus,
      systemPrompt, systemPromptStatus,
      contextPreview, contextStatus, contextTrimMode, contextKeepMessages, contextTriggerMaxMessages, contextRetainTurns, contextIncludeAfter,
      contextSeedKind, contextSeedContent, contextSeedMode,
      streamingText, isStreaming, currentRunId, currentRunStatus, runStatusLabel, autoReconnect,
      streamingThinking,
      latestToolChainReport, latestTraceDetails, formatToolChainIds, toolChainStatusText, toolChainIssueCount,
      connect, disconnect, sendChat, retrySession, insertChatMessage, cancelCurrentRun, sendRaw, sendToolResult,
      defineClientTool, handleToolCallRequest,
      loadSessions,
      loadArchivedSessions,
      toggleArchivedSessions,
      selectSession, createNewSession, closeSession, archiveSession, unarchiveSession, deleteSession, forkSession,
      registerPresetTool, registerPresetTools,
      saveProvider, loadProvider, applyTemplate,
      saveSystemPrompt, loadSystemPrompt, loadTools,
      loadRuntimeSessions,
      loadContextPreview, setTrimNone, setContextTrim,
      excludeSingleMessage, copyMessageId, saveContextSeed, deleteContextSeed, clearContextSeedsByKind, clearAllContextSeeds, contextMessageText,
      contextCommandName, contextModeLabel, seedKindLabel,
      loadFullMessages, onFullMessagesScroll, roleLabel, roleColor, formatMessageTime,
      filteredEvents, filteredRaw, statsExpanded, showPresetToolsModal, renderMarkdown,
      showApiKey, formatTimeoutLabel, askUserQuestionState,
      addImageFiles, removePendingImage, clearPendingImages, handlePaste, handleFileSelect, triggerFileSelect,
      addAudioFiles, removePendingAudio, clearPendingAudio, triggerAudioFileSelect, handleAudioFileSelect, toggleRecording, formatDuration,
      mobileView, switchMobileView, mobileSessionOpen, mobileMoreOpen, currentSessionTitle, formatSize,
    };
  }
}).mount('#app');
