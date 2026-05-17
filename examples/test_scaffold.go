// AI Scaffold — 完整单文件测试示例
// 运行: go run examples/test_scaffold.go
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/ai-scaffold/go/config"
	"github.com/ai-scaffold/go/hooks"
	"github.com/ai-scaffold/go/runtime"
	"github.com/ai-scaffold/go/tools"
)

func main() {
	fmt.Println("══════════════════════════════════════════════════")
	fmt.Println("  AI Scaffold — Go 完整测试示例")
	fmt.Println("══════════════════════════════════════════════════")

	// ── 1. 配置 ──
	cfg := config.DefaultRuntimeConfig()
	cfg.Provider.Name = "deepseek"
	cfg.Provider.Protocol = config.ProtocolOpenAI
	cfg.Provider.BaseURL = "https://api.deepseek.com"
	cfg.Provider.APIKey = os.Getenv("OPENAI_API_KEY")
	if cfg.Provider.APIKey == "" {
		cfg.Provider.APIKey = os.Getenv("DEEPSEEK_API_KEY")
	}
	cfg.Provider.Model = "deepseek-chat"
	cfg.Provider.MaxTokens = 4096
	cfg.Provider.Temperature = 0.7
	cfg.Provider.SupportsToolUse = true

	// 工具能力开关
	cfg.Tools.Capabilities = []config.ToolCapability{
		{Key: "web.search", Enabled: true, Label: "网络搜索"},
		{Key: "math.calc", Enabled: true, Label: "数学计算"},
		{Key: "file.read", Enabled: true, Label: "文件读取"},
		{Key: "weather.query", Enabled: false, Label: "天气查询"},
	}
	cfg.Tools.MaxToolRounds = 10

	cfg.SystemPrompt = "你是一个有用的 AI 助手。你可以使用工具来帮助用户完成任务。请用中文回答。"

	// ── 2. 创建脚手架 ──
	s := runtime.New(cfg)

	// ── 3. 注册工具 ──
	registerTools(s)

	// ── 4. 注册钩子 ──
	registerHooks(s)

	// ── 5. 显示已注册信息 ──
	fmt.Printf("\n已注册工具: %d 个\n", s.ToolManager.Count())
	for _, t := range s.ToolManager.ListAll() {
		fmt.Printf("  - %s: %s\n", t.Name, truncate(t.Description, 50))
	}

	// ── 6. 测试对话 ──
	apiKey := cfg.Provider.APIKey
	if apiKey != "" {
		runChat(s, "你好，请介绍一下你自己。", "test_1")
		runChat(s, "帮我搜索一下 Python 3.12 的新特性", "test_1")
		runChat(s, "计算一下 (15 + 27) * 3 - 8", "test_1")
	} else {
		fmt.Println("\n⚠️  未设置 API Key，使用模拟模式")
		testToolsDirectly(s)
	}

	// 清理
	s.ClearSession("test_1")
	fmt.Println("\n✅ 测试完成")
}

// ── 注册工具 ──────────────────────────────────────────────────────────

func registerTools(s *runtime.Scaffold) {
	// 搜索工具
	s.RegisterTool(&tools.Tool{
		Name:        "search",
		Description: "在互联网上搜索信息。当用户需要查找资料、新闻、知识时使用。",
		InputSchema: map[string]interface{}{
			"type": "object",
			"properties": map[string]interface{}{
				"query": map[string]interface{}{"type": "string", "description": "搜索关键词"},
			},
			"required": []string{"query"},
		},
		CapabilityKey: "web.search",
		Category:      "web",
		Function: func(input map[string]interface{}) (string, error) {
			query, _ := input["query"].(string)
			return fmt.Sprintf("搜索结果: 关于\"%s\"，找到以下内容:\n1. %s的相关文档\n2. %s的使用教程", query, query, query), nil
		},
	})

	// 计算器工具
	s.RegisterTool(&tools.Tool{
		Name:        "calculator",
		Description: "执行数学计算。支持基本四则运算。",
		InputSchema: map[string]interface{}{
			"type": "object",
			"properties": map[string]interface{}{
				"expression": map[string]interface{}{"type": "string", "description": "数学表达式"},
			},
			"required": []string{"expression"},
		},
		CapabilityKey: "math.calc",
		Category:      "math",
		Function: func(input map[string]interface{}) (string, error) {
			expr, _ := input["expression"].(string)
			return fmt.Sprintf("计算结果: %s = (模拟值 42)", expr), nil
		},
	})

	// 文件读取工具
	s.RegisterTool(&tools.Tool{
		Name:        "file_read",
		Description: "读取文件内容。",
		InputSchema: map[string]interface{}{
			"type": "object",
			"properties": map[string]interface{}{
				"path": map[string]interface{}{"type": "string", "description": "文件路径"},
			},
			"required": []string{"path"},
		},
		CapabilityKey: "file.read",
		Category:      "file",
		Function: func(input map[string]interface{}) (string, error) {
			path, _ := input["path"].(string)
			return fmt.Sprintf("文件内容 (%s):\n这是一段模拟的文件内容。", path), nil
		},
	})

	// 天气工具（能力关闭状态）
	s.RegisterTool(&tools.Tool{
		Name:        "weather",
		Description: "查询指定城市的天气信息。",
		InputSchema: map[string]interface{}{
			"type": "object",
			"properties": map[string]interface{}{
				"city": map[string]interface{}{"type": "string", "description": "城市名称"},
			},
			"required": []string{"city"},
		},
		CapabilityKey: "weather.query", // 此能力默认关闭
		Category:      "info",
		Function: func(input map[string]interface{}) (string, error) {
			city, _ := input["city"].(string)
			return fmt.Sprintf("%s 今日天气: 晴, 25°C, 微风", city), nil
		},
	})
}

// ── 注册钩子 ──────────────────────────────────────────────────────────

func registerHooks(s *runtime.Scaffold) {
	s.RegisterHook(hooks.EventBeforeChat, "log_before", func(ctx *hooks.EventContext) error {
		fmt.Printf("  [Hook] BeforeChat | session=%s | msg=%s\n",
			ctx.SessionID, truncate(ctx.GetString("message"), 50))
		return nil
	})

	s.RegisterHook(hooks.EventAfterChat, "log_after", func(ctx *hooks.EventContext) error {
		fmt.Printf("  [Hook] AfterChat  | %s\n", truncate(ctx.GetString("content"), 80))
		return nil
	})

	s.RegisterHook(hooks.EventToolBefore, "log_tool_before", func(ctx *hooks.EventContext) error {
		fmt.Printf("  [Hook] ToolBefore | calling: %s\n", ctx.GetString("tool_name"))
		return nil
	})

	s.RegisterHook(hooks.EventToolAfter, "log_tool_after", func(ctx *hooks.EventContext) error {
		isErr, _ := ctx.Get("is_error")
		status := "OK"
		if isErr == true {
			status = "ERROR"
		}
		fmt.Printf("  [Hook] ToolAfter  | %s -> [%s] %s\n",
			ctx.GetString("tool_name"), status, truncate(ctx.GetString("result"), 60))
		return nil
	})

	s.RegisterHook(hooks.EventOnError, "log_error", func(ctx *hooks.EventContext) error {
		if ctx.Err != nil {
			fmt.Printf("  [Hook] OnError    | %s\n", ctx.Err)
		}
		return nil
	})

	s.RegisterHook(hooks.EventOnRetry, "log_retry", func(ctx *hooks.EventContext) error {
		fmt.Printf("  [Hook] OnRetry    | attempt #%v\n", ctx.GetString("attempt"))
		return nil
	})
}

// ── 对话测试 ──────────────────────────────────────────────────────────

func runChat(s *runtime.Scaffold, message, sessionID string) {
	fmt.Printf("\n%s\n用户: %s\n%s\n", "────────────────────────────────", message, "────────────────────────────────")

	resp, err := s.Chat(context.Background(), runtime.ChatOptions{
		SessionID: sessionID,
		Message:   message,
	})
	if err != nil {
		fmt.Printf("\n❌ 错误: %s\n", err)
		return
	}

	fmt.Printf("\n助手: %s\n", resp.Content)
	fmt.Printf("  Token: input=%d output=%d total=%d\n",
		resp.Usage.InputTokens, resp.Usage.OutputTokens, resp.Usage.Total())
	printStats(resp.Stats)
}

func testToolsDirectly(s *runtime.Scaffold) {
	fmt.Println("\n── 模拟模式: 直接测试工具执行 ──")

	result, _ := s.ToolManager.Execute("search", map[string]interface{}{"query": "Python 3.12"}, nil)
	fmt.Printf("  search('Python 3.12') => %s\n", truncate(result, 80))

	result, _ = s.ToolManager.Execute("calculator", map[string]interface{}{"expression": "(15+27)*3-8"}, nil)
	fmt.Printf("  calculator => %s\n", result)

	result, _ = s.ToolManager.Execute("file_read", map[string]interface{}{"path": "/etc/hosts"}, nil)
	fmt.Printf("  file_read => %s\n", truncate(result, 80))

	// 天气（能力关闭，但仍可直接执行 — 能力开关只影响 AI 可见性）
	result, _ = s.ToolManager.Execute("weather", map[string]interface{}{"city": "北京"}, nil)
	fmt.Printf("  weather => %s\n", result)

	// 不存在的工具
	_, err := s.ToolManager.Execute("nonexistent", map[string]interface{}{}, nil)
	fmt.Printf("  nonexistent => ❌ %s\n", err)
}

// ── 辅助函数 ──────────────────────────────────────────────────────────

func printStats(stats interface{ CompressionRatio() float64 }) {
	// 简化打印
	fmt.Printf("  [统计] 压缩比: %.2f%%\n", 0.0)
}

func truncate(s string, maxLen int) string {
	runes := []rune(s)
	if len(runes) <= maxLen {
		return s
	}
	return string(runes[:maxLen]) + "..."
}
