---
title: 开发者手册
description: 面向开发 fileOS 或在 fileOS 之上开发的人。
---

本手册面向两类开发者：开发 fileOS 本身的人，以及为它编写 App 和引擎的人。

## 在 fileOS 之上开发

内核之上的一切都是放在 `.app` 文件夹里的 WebAssembly 组件：App，以及决定节点规则的引擎。

- [编写 App 和引擎](apps/)：接口、SDK、`.app` 文件夹和 manifest。
- 接口定义在 [`wit/fos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fos.wit)。

## 开发 fileOS

- [参与贡献](contributing/)：如何搭建环境、改动必须遵守的规则，以及如何提交。
- [架构](fileos/)：fileOS 是怎样构建的，以及为什么。
- [值得学习的项目](prior-art/)：我们研究的项目，以及从每个项目学到什么。

原则优先。见 [README](https://github.com/nmnmcc/fileOS/blob/main/README.zh-CN.md) 的“怎样构成”：内核守住它的契约，App 是用户空间，内核插件绝不改变 App 看到的东西。我们用它评判每一个改动。
