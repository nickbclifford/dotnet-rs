using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Threading.Tasks.ValueTask")]
public readonly struct ValueTask
{
    [UsedImplicitly] private readonly Task? _task;

    public ValueTask(Task task)
    {
        _task = task ?? throw new ArgumentNullException(nameof(task));
    }

    public bool IsCompleted => (_task ?? Task.CompletedTask).IsCompleted;

    public Task AsTask() => _task ?? Task.CompletedTask;

    public ValueTaskAwaiter GetAwaiter() => new(this);
}

[Stub(InPlaceOf = "System.Threading.Tasks.ValueTask`1")]
public readonly struct ValueTask<TResult>
{
    [UsedImplicitly] private readonly Task<TResult>? _task;
    [UsedImplicitly] private readonly TResult _result;
    [UsedImplicitly] private readonly bool _hasResult;

    public ValueTask(Task<TResult> task)
    {
        _task = task ?? throw new ArgumentNullException(nameof(task));
        _result = default!;
        _hasResult = false;
    }

    public ValueTask(TResult result)
    {
        _task = null;
        _result = result;
        _hasResult = true;
    }

    public bool IsCompleted => _hasResult || _task is null || _task.IsCompleted;

    public TResult Result
    {
        get
        {
            if (_hasResult)
            {
                return _result;
            }
            if (_task is null)
            {
                return default!;
            }
            return _task.Result;
        }
    }

    public Task<TResult> AsTask()
    {
        if (_hasResult)
        {
            return Task.FromResult<TResult>(_result);
        }
        return _task ?? Task.FromResult<TResult>(default!);
    }

    public ValueTaskAwaiter<TResult> GetAwaiter() => new(this);
}
