//! Rust异步运行时Demo
//!
//! 所谓的异步运行时（以下简称Runtime），其实和操作系统的调度器（以下简称Scheduler）十分类似。
//! - Scheduler调度`Process` <=> Runtime调度`Task`
//! - Scheduler的`Process`主要有`Ready`/`Running`/`Waiting`三个状态 <=> Runtime的`Task`也有`Ready`/`Running`/`Waiting`状态
//!   - Scheduler有`ReadyQueue`存储`Ready/Running`的`Process` <=> Runtime也有`ReadyQueue`存储`Ready`的`Task`（Demo中为`ex.local_queue`）
//!   - 那Runtime里`Running`的`Task`去哪了？其实并没有被删除，他还在内存中，等待poll的返回结果。如果真的完成了就删掉，如果还没完成就放在WatingQueue中。
//!   - Scheduler有`WatingQueue`存储等待IO的`Process` <=> Runtime也有`WatingQueue`存储等待IO的`Task`（Demo中为`ex.reactor.waker_mapping`）
//! - Scheduler的`Process`如果进入`Waiting`状态，要等待`Interrupt`将其唤醒 <=> Runtime的`Task`如果进入`Waiting`状态，要等待操作系统的回调将其唤醒
//!   - 中断是各种各样的
//!   - 操作系统提供的回调有`select(), poll(), epoll()` (I/O多路复用机制，让你知道一堆被IO阻塞的Task哪个就绪了)。Runtime要把它们进行封装
//!
//! ![](https://www.ihcblog.com/images/062812972a4bd0e09fdce4bff62204bc1973e5ac.png)

#![allow(unused)]

pub mod executor;
pub mod tcp;

mod reactor;
