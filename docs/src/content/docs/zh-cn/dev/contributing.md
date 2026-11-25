---
title: 参与贡献
description: 搭建开发环境并提交修改。
---

讨论和代码同样受欢迎：可以开 issue 质疑某条原则、某个设计决定，或本页的
任何内容。完整规则见仓库中的 `CONTRIBUTING.zh-CN.md`，本页是精简版。

## 环境搭建

我们推荐 [devenv](https://devenv.sh) 加 [direnv](https://direnv.net)，但任何
环境都可以；是否接受修改只由 CI 决定。

```sh
direnv allow     # 在仓库中执行一次
check            # 推送前：格式、lint、测试、许可证
budget           # 编译时间预算
docs             # 预览本站
```

不用 Nix：用 rustup 安装 Rust（`rust-toolchain.toml` 会选择版本），运行
`cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`，并安装
[uv](https://docs.astral.sh/uv/) 来运行脚本（`uv sync`）。编写文档需要
Node.js 22.23.4 或更高版本：在 `docs/` 中运行 `npm install` 和
`npm run dev`。

## 编译时间是一项功能

普通笔记本电脑应当足以参与开发，所以 CI 对每个拉取请求检查构建预算，基准
是一台 4 核、8 GB 内存的笔记本：

| 指标                     | 预算    |
| ------------------------ | ------- |
| 整个工作区的干净调试构建 | ≤ 180 s |
| 修改叶子 crate 后重建    | ≤ 5 s   |
| 修改 `hs-core` 后重建    | ≤ 20 s  |

超出预算的拉取请求与测试失败的同等对待。

每个新依赖都要审查。说明为什么需要它，以及它让干净构建增加了多少；按构建
时间、二进制大小和它带来的东西（第二套加密库或 HTTP 栈、重型过程宏、构建
脚本、C 或 C++）判断，而不是按 crate 数量。在根 `Cargo.toml` 中声明一次，
并设置 `default-features = false`。禁止 `async-trait`；不接受需要 cmake 的
C 或 C++。

## 代码的形态

- crate 小，依赖图浅。依赖只能向下；`hs-core` 不依赖其他 homeserver
  crate，`hs` 保持轻薄。
- 标记为 `portable = true` 的 crate 必须能为 `wasm32-unknown-unknown`
  构建：其中不能有线程、文件、套接字和时钟。
- 协议逻辑是 sans-IO 的，同一份代码可以在确定性模拟（`hs-sim`）中运行。
- 机制而非策略：当代码要决定"应该发生什么"（帮谁、分享什么、保留什么）时，
  把它做成带默认值、可替换的策略。

crate 布局见[架构](../architecture/#crate)。

## 许可证与签署

homeserver 采用 AGPL-3.0-only；`hs-plugin-api` 采用 MIT OR Apache-2.0，
以便插件可以使用任何许可证。每个源文件以 SPDX 头开始。没有 CLA：你保留
自己的版权。

每个提交都必须按
[Developer Certificate of Origin](https://developercertificate.org/) 签署：
`git commit -s`。

## 文档

本站用 [Astro Starlight](https://starlight.astro.build) 构建，位于 `docs/`。
英文页面在 `docs/src/content/docs/en-us/`，简体中文页面在
`docs/src/content/docs/zh-cn/`，文件名相同。页面按读者分组：`user/`、
`admin/` 和 `dev/`，每组是侧栏顶部切换器中的一个主题。新页面要放进两种
语言的对应目录，并加入 `docs/astro.config.mjs` 的 `topics` 列表。

为该手册的读者写作。用户手册不假定读者了解服务器；管理手册假定读者会用
终端；只有开发者手册才谈 crate 和协议。描述当前版本实际做到的事；尚未实现的
设计属于[架构](../architecture/)。
