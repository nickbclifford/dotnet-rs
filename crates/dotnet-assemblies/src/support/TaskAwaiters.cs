using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Runtime.CompilerServices.TaskAwaiter")]
public readonly struct TaskAwaiter : ICriticalNotifyCompletion
{
    [UsedImplicitly] private readonly Task _task;

    internal TaskAwaiter(Task task)
    {
        _task = task ?? throw new ArgumentNullException(nameof(task));
    }

    public bool IsCompleted => _task.IsCompleted;

    public void OnCompleted(Action continuation) => _task.OnCompleted(continuation);

    public void UnsafeOnCompleted(Action continuation) => _task.OnCompleted(continuation);

    public void GetResult() => _task.GetResult();
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.TaskAwaiter`1")]
public readonly struct TaskAwaiter<TResult> : ICriticalNotifyCompletion
{
    [UsedImplicitly] private readonly Task<TResult> _task;

    internal TaskAwaiter(Task<TResult> task)
    {
        _task = task ?? throw new ArgumentNullException(nameof(task));
    }

    public bool IsCompleted => _task.IsCompleted;

    public void OnCompleted(Action continuation) => _task.OnCompleted(continuation);

    public void UnsafeOnCompleted(Action continuation) => _task.OnCompleted(continuation);

    public TResult GetResult() => _task.GetResultCore();
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.ValueTaskAwaiter")]
public readonly struct ValueTaskAwaiter : ICriticalNotifyCompletion
{
    [UsedImplicitly] private readonly ValueTask _valueTask;

    internal ValueTaskAwaiter(ValueTask valueTask)
    {
        _valueTask = valueTask;
    }

    public bool IsCompleted => _valueTask.IsCompleted;

    public void OnCompleted(Action continuation) => _valueTask.AsTask().OnCompleted(continuation);

    public void UnsafeOnCompleted(Action continuation) => _valueTask.AsTask().OnCompleted(continuation);

    public void GetResult() => _valueTask.AsTask().GetResult();
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.ValueTaskAwaiter`1")]
public readonly struct ValueTaskAwaiter<TResult> : ICriticalNotifyCompletion
{
    [UsedImplicitly] private readonly ValueTask<TResult> _valueTask;

    internal ValueTaskAwaiter(ValueTask<TResult> valueTask)
    {
        _valueTask = valueTask;
    }

    public bool IsCompleted => _valueTask.IsCompleted;

    public void OnCompleted(Action continuation) => _valueTask.AsTask().OnCompleted(continuation);

    public void UnsafeOnCompleted(Action continuation) => _valueTask.AsTask().OnCompleted(continuation);

    public TResult GetResult() => _valueTask.Result;
}
