use crate::reactor::Reactor;
use futures::{future::LocalBoxFuture, Future, FutureExt};
use std::{
    cell::RefCell,
    collections::VecDeque,
    marker::PhantomData,
    mem,
    pin::Pin,
    rc::Rc,
    task::{Context, RawWaker, RawWakerVTable, Waker},
};
scoped_tls::scoped_thread_local!(pub(crate) static EX: Executor);

/// `Executor`负责`Task`的调度和执行
pub struct Executor {
    /// 等待调度的`Task`队列
    local_queue: TaskQueue,
    pub(crate) reactor: Rc<RefCell<Reactor>>,

    /// Make sure the type is `!Send` and `!Sync`.
    _marker: PhantomData<Rc<()>>,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    /// 创建一个新的`Executor`
    pub fn new() -> Self {
        Self {
            local_queue: TaskQueue::default(),
            reactor: Rc::new(RefCell::new(Reactor::default())),

            _marker: PhantomData,
        }
    }

    /// 一个`async fn`可以认为是一个`Future`
    /// `spawn`将`Future`加入调度队列
    pub fn spawn(fut: impl Future<Output = ()> + 'static) {
        let t = Rc::new(Task {
            future: RefCell::new(fut.boxed_local()),
        });
        EX.with(|ex| ex.local_queue.push(t));
    }

    /// 创建一个 dummy_waker，这个 waker 其实啥事不做。
    /// (in loop)poll 传入的 future，检查是否 ready，如果 ready 就返回，结束 block_on。
    /// (in loop)循环处理 TaskQueue 中的所有 Task：构建它对应的 Waker 然后 poll 它。
    /// (in loop)这时已经没有待执行的任务了，可能主 future 已经 ready 了，也可能都在等待 IO。所以再次检查主 future，如果 ready 就返回。
    /// (in loop)既然所有人都在等待 IO，那就 reactor.wait()。这时 reactor 会陷入 syscall 等待至少一个 IO 可执行，然后唤醒对应 Task，会向 TaskQueue 里推任务。
    pub fn block_on<F, T, O>(&self, f: F) -> O
    where
        F: Fn() -> T,
        T: Future<Output = O> + 'static,
    {
        let _waker = waker_fn::waker_fn(|| {});
        let cx = &mut Context::from_waker(&_waker);

        EX.set(self, || {
            let fut = f();
            pin_utils::pin_mut!(fut);
            loop {
                // return if the outer future is ready
                if let std::task::Poll::Ready(t) = fut.as_mut().poll(cx) {
                    break t;
                }

                // consume all tasks
                while let Some(t) = self.local_queue.pop() {
                    let future = t.future.borrow_mut();
                    let w = waker(t.clone());
                    let mut context = Context::from_waker(&w);
                    let _ = Pin::new(future).as_mut().poll(&mut context);
                }

                // no task to execute now, it may ready
                if let std::task::Poll::Ready(t) = fut.as_mut().poll(cx) {
                    break t;
                }

                // block for io
                self.reactor.borrow_mut().wait();
            }
        })
    }
}

/// 存储`Task`的队列
pub struct TaskQueue {
    queue: RefCell<VecDeque<Rc<Task>>>,
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskQueue {
    /// 创建一个新的`TaskQueue`
    pub fn new() -> Self {
        const DEFAULT_TASK_QUEUE_SIZE: usize = 4096;
        Self::new_with_capacity(DEFAULT_TASK_QUEUE_SIZE)
    }

    /// 创建一个新的`TaskQueue`
    pub fn new_with_capacity(capacity: usize) -> Self {
        Self {
            queue: RefCell::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// 添加一个`Task`
    pub(crate) fn push(&self, runnable: Rc<Task>) {
        println!("add task");
        self.queue.borrow_mut().push_back(runnable);
    }

    /// 删除第一个`Task`
    pub(crate) fn pop(&self) -> Option<Rc<Task>> {
        println!("remove task");
        self.queue.borrow_mut().pop_front()
    }
}

/// `Task`是对`Future`的一个简单封装
pub struct Task {
    future: RefCell<LocalBoxFuture<'static, ()>>,
}

/// 创建一个和`Task`关联的`Waker`, 当`Task`准备好执行的时候, 调用`Waker`提供的`wake`和`wake_by_ref`方法
fn waker(wake: Rc<Task>) -> Waker {
    let ptr = Rc::into_raw(wake) as *const ();
    let vtable = &Helper::VTABLE;
    unsafe { Waker::from_raw(RawWaker::new(ptr, vtable)) }
}

impl Task {
    /// 唤醒`Task`, 添加到调度队列中等待调度
    fn wake_(self: Rc<Self>) {
        Self::wake_by_ref_(&self)
    }

    /// 唤醒`Task`, 添加到调度队列中等待调度
    fn wake_by_ref_(self: &Rc<Self>) {
        EX.with(|ex| ex.local_queue.push(self.clone()));
    }
}

struct Helper;

impl Helper {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        Self::clone_waker,
        Self::wake,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    unsafe fn clone_waker(data: *const ()) -> RawWaker {
        increase_refcount(data);
        let vtable = &Self::VTABLE;
        RawWaker::new(data, vtable)
    }

    unsafe fn wake(ptr: *const ()) {
        let rc = Rc::from_raw(ptr as *const Task);
        rc.wake_();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let rc = mem::ManuallyDrop::new(Rc::from_raw(ptr as *const Task));
        rc.wake_by_ref_();
    }

    unsafe fn drop_waker(ptr: *const ()) {
        drop(Rc::from_raw(ptr as *const Task));
    }
}

#[allow(clippy::redundant_clone)] // The clone here isn't actually redundant.
unsafe fn increase_refcount(data: *const ()) {
    // Retain Rc, but don't touch refcount by wrapping in ManuallyDrop
    let rc = mem::ManuallyDrop::new(Rc::<Task>::from_raw(data as *const Task));
    // Now increase refcount, but don't drop new refcount either
    let _rc_clone: mem::ManuallyDrop<_> = rc.clone();
}
