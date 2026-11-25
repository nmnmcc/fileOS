---
title: 空间与条目
description: 在 homeserver 上添加、读取、修改和删除内容。
---

命令先写空间（名称或 ID），再写 `类型/ID` 形式的条目。类型用来描述条目是
什么：`note`、`contact`、`event`、`recipe`。搜索和时间线能识别几个常用
字段，见下文。

## 列出和读取

```sh
hs ls notes                 # 空间 notes 里有什么
hs ls notes note            # 只看笔记
hs ls notes --all           # 全部，逐页列出
hs get notes note/shopping
hs get photos file/3f2a… -o beach.jpg
```

## 写入

`hs put` 从文件或标准输入创建或替换一个条目：

```sh
echo '{"title": "购物", "text": "牛奶、鸡蛋"}' | hs put notes note/shopping
hs put notes recipe/bread bread.md
```

媒体类型根据文件名判断，没有文件名时视为 JSON；可以用 `--media-type`
指定。为了安全地修改：`--new` 在条目已存在时失败，`--if-match ETAG` 在
别人改过之后失败。

两种更简单的修改方式：

```sh
hs edit notes note/shopping                     # 打开 $EDITOR；新 ID 从空白开始
hs patch notes note/shopping '{"done": true}'   # 修改 JSON 条目的部分字段
```

`hs patch` 接受 JSON merge patch：给出的字段被设置，设为 `null` 的字段被
删除，其余保持不变。

## 搜索和时间线能识别的字段

```json
{
  "title": "周末",
  "text": "和妈妈去人民公园散步",
  "time": "2024-05-01T09:30:00Z",
  "place": { "lat": 31.23, "lon": 121.47 },
  "links": [{ "rel": "with", "href": "person/mom" }]
}
```

- `title`（或 `name`）和 `text` 会被搜索。
- `time` 让条目出现在时间线上。
- `place` 让 [`hs near`](../finding/) 能找到它。
- `links` 把它和人、地点及其他条目关联起来，供
  [`hs related`](../finding/) 使用。

## 文件和照片

```sh
hs upload photos IMG_0001.jpg IMG_0002.jpg
```

每个文件成为一个以内容命名的 `file/…` 条目，所以同一个文件上传两次不会有
任何变化。照片的时间、地点和相机从 EXIF 读取，JPEG 和 PNG 会生成缩略图。

## 删除

```sh
hs rm notes note/shopping
hs rm notes note/shopping --erase
```

普通的 `rm` 删除条目，但像其他修改一样保留历史。`--erase` 还会从所有关注
这个空间的成员的日志中抹去它以前的版本。别人已经复制走的内容无法收回。
