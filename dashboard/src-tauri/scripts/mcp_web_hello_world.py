"""MCP Server: web_hello_world — WebページからHello Worldを取得して表示するMCPサーバー"""
import asyncio
import json

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

app = Server("web_hello_world")

@app.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="get_hello_world",
            description="WebからHello Worldを取得して表示します",
            inputSchema={
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Hello Worldを表示するWebページのURL（省略可）",
                        "default": "https://example.com"
                    }
                },
                "required": []
            }
        ),
        Tool(
            name="create_hello_world_page",
            description="Hello Worldを表示する簡単なHTMLページを作成します",
            inputSchema={
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "ページのタイトル",
                        "default": "Hello World Page"
                    },
                    "message": {
                        "type": "string",
                        "description": "表示するメッセージ",
                        "default": "Hello World!"
                    }
                },
                "required": []
            }
        )
    ]

@app.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    if name == "get_hello_world":
        import httpx
        url = arguments.get("url", "https://example.com")
        
        try:
            async with httpx.AsyncClient() as client:
                response = await client.get(url, timeout=10.0)
                response.raise_for_status()
                
                # ページからHello Worldを探す
                content = response.text
                if "Hello" in content or "hello" in content or "World" in content or "world" in content:
                    result = {
                        "success": True,
                        "url": url,
                        "status_code": response.status_code,
                        "content_preview": content[:500] + "..." if len(content) > 500 else content,
                        "message": "ページからHello World関連のコンテンツが見つかりました"
                    }
                else:
                    result = {
                        "success": True,
                        "url": url,
                        "status_code": response.status_code,
                        "content_preview": content[:500] + "..." if len(content) > 500 else content,
                        "message": "ページを取得しましたが、Hello Worldは見つかりませんでした"
                    }
                    
        except Exception as e:
            result = {
                "success": False,
                "url": url,
                "error": str(e),
                "message": "ページの取得に失敗しました"
            }
        
        return [TextContent(type="text", text=json.dumps(result, ensure_ascii=False, indent=2))]
    
    elif name == "create_hello_world_page":
        title = arguments.get("title", "Hello World Page")
        message = arguments.get("message", "Hello World!")
        
        html_content = f"""<!DOCTYPE html>
<html lang="ja">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <style>
        body {{
            font-family: Arial, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background-color: #f0f0f0;
        }}
        .container {{
            text-align: center;
            padding: 40px;
            background-color: white;
            border-radius: 10px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }}
        h1 {{
            color: #333;
            font-size: 3em;
            margin-bottom: 20px;
        }}
        p {{
            color: #666;
            font-size: 1.5em;
        }}
        .timestamp {{
            color: #999;
            font-size: 0.9em;
            margin-top: 30px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>{message}</h1>
        <p>これはMCPサーバーで生成されたHello Worldページです</p>
        <div class="timestamp">
            生成日時: {datetime.datetime.now().strftime('%Y年%m月%d日 %H:%M:%S')}
        </div>
    </div>
</body>
</html>"""
        
        result = {
            "success": True,
            "title": title,
            "message": message,
            "html_content": html_content,
            "instructions": "このHTMLをファイルに保存してブラウザで開くと、Hello Worldページが表示されます"
        }
        
        return [TextContent(type="text", text=json.dumps(result, ensure_ascii=False, indent=2))]
    
    raise ValueError(f"Unknown tool: {name}")

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())

if __name__ == "__main__":
    asyncio.run(main())
