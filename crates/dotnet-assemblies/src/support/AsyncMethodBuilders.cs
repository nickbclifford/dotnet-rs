using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Runtime.CompilerServices.AsyncTaskMethodBuilder")]
public struct AsyncTaskMethodBuilder
{
    [UsedImplicitly] private Task? _task;

    public static AsyncTaskMethodBuilder Create() => new();

    public Task Task => EnsureTask();

    public void Start<TStateMachine>(ref TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        _ = EnsureTask();
        stateMachine.MoveNext();
    }

    public void SetStateMachine(IAsyncStateMachine stateMachine) { }

    public void SetResult() => EnsureTask().TrySetResult();

    public void SetException(Exception exception) => EnsureTask().TrySetException(exception);

    public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
        where TAwaiter : INotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.OnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(
        ref TAwaiter awaiter,
        ref TStateMachine stateMachine
    )
        where TAwaiter : ICriticalNotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.UnsafeOnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    private Task EnsureTask() => _task ??= new Task();

    private static Action CreateMoveNextContinuation<TStateMachine>(TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        object boxedStateMachine = stateMachine!;
        return () => ((IAsyncStateMachine)boxedStateMachine).MoveNext();
    }
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.AsyncTaskMethodBuilder`1")]
public struct AsyncTaskMethodBuilder<TResult>
{
    [UsedImplicitly] private Task<TResult>? _task;

    public static AsyncTaskMethodBuilder<TResult> Create() => new();

    public Task<TResult> Task => EnsureTask();

    public void Start<TStateMachine>(ref TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        _ = EnsureTask();
        stateMachine.MoveNext();
    }

    public void SetStateMachine(IAsyncStateMachine stateMachine) { }

    public void SetResult(TResult result) => EnsureTask().TrySetResult(result);

    public void SetException(Exception exception) => EnsureTask().TrySetException(exception);

    public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
        where TAwaiter : INotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.OnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(
        ref TAwaiter awaiter,
        ref TStateMachine stateMachine
    )
        where TAwaiter : ICriticalNotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.UnsafeOnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    private Task<TResult> EnsureTask() => _task ??= new Task<TResult>();

    private static Action CreateMoveNextContinuation<TStateMachine>(TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        object boxedStateMachine = stateMachine!;
        return () => ((IAsyncStateMachine)boxedStateMachine).MoveNext();
    }
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.AsyncValueTaskMethodBuilder")]
public struct AsyncValueTaskMethodBuilder
{
    [UsedImplicitly] private Task? _task;

    public static AsyncValueTaskMethodBuilder Create() => new();

    public ValueTask Task => new(EnsureTask());

    public void Start<TStateMachine>(ref TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        _ = EnsureTask();
        stateMachine.MoveNext();
    }

    public void SetStateMachine(IAsyncStateMachine stateMachine) { }

    public void SetResult() => EnsureTask().TrySetResult();

    public void SetException(Exception exception) => EnsureTask().TrySetException(exception);

    public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
        where TAwaiter : INotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.OnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(
        ref TAwaiter awaiter,
        ref TStateMachine stateMachine
    )
        where TAwaiter : ICriticalNotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.UnsafeOnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    private Task EnsureTask() => _task ??= new Task();

    private static Action CreateMoveNextContinuation<TStateMachine>(TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        object boxedStateMachine = stateMachine!;
        return () => ((IAsyncStateMachine)boxedStateMachine).MoveNext();
    }
}

[Stub(InPlaceOf = "System.Runtime.CompilerServices.AsyncValueTaskMethodBuilder`1")]
public struct AsyncValueTaskMethodBuilder<TResult>
{
    [UsedImplicitly] private Task<TResult>? _task;

    public static AsyncValueTaskMethodBuilder<TResult> Create() => new();

    public ValueTask<TResult> Task => new(EnsureTask());

    public void Start<TStateMachine>(ref TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        _ = EnsureTask();
        stateMachine.MoveNext();
    }

    public void SetStateMachine(IAsyncStateMachine stateMachine) { }

    public void SetResult(TResult result) => EnsureTask().TrySetResult(result);

    public void SetException(Exception exception) => EnsureTask().TrySetException(exception);

    public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
        where TAwaiter : INotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.OnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(
        ref TAwaiter awaiter,
        ref TStateMachine stateMachine
    )
        where TAwaiter : ICriticalNotifyCompletion
        where TStateMachine : IAsyncStateMachine
    {
        awaiter.UnsafeOnCompleted(CreateMoveNextContinuation(stateMachine));
    }

    private Task<TResult> EnsureTask() => _task ??= new Task<TResult>();

    private static Action CreateMoveNextContinuation<TStateMachine>(TStateMachine stateMachine)
        where TStateMachine : IAsyncStateMachine
    {
        object boxedStateMachine = stateMachine!;
        return () => ((IAsyncStateMachine)boxedStateMachine).MoveNext();
    }
}
