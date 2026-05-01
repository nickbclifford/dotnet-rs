using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Threading.Tasks.Task")]
public class Task
{
    [UsedImplicitly] private bool _isCompleted;
    [UsedImplicitly] private Exception? _exception;
    [UsedImplicitly] private Action? _continuation;

    internal Task() { }
    internal Task(bool completed)
    {
        _isCompleted = completed;
    }

    public static Task CompletedTask { get; } = new(completed: true);

    public static Task<TResult> FromResult<TResult>(TResult result) => Task<TResult>.FromResult(result);

    public static Task FromException(Exception exception)
    {
        ArgumentNullException.ThrowIfNull(exception);
        var task = new Task();
        task.TrySetException(exception);
        return task;
    }

    public bool IsCompleted => _isCompleted;
    public bool IsCompletedSuccessfully => _isCompleted && _exception is null;
    public bool IsFaulted => _exception is not null;
    public AggregateException? Exception => _exception is null ? null : new AggregateException(_exception);

    public TaskAwaiter GetAwaiter() => new(this);

    public void Wait() => GetResult();

    internal void OnCompleted(Action continuation)
    {
        ArgumentNullException.ThrowIfNull(continuation);
        Action? runImmediately = null;
        lock (this)
        {
            if (_isCompleted)
            {
                runImmediately = continuation;
            }
            else
            {
                _continuation += continuation;
            }
        }
        runImmediately?.Invoke();
    }

    internal void TrySetResult()
    {
        Action? continuation = null;
        lock (this)
        {
            if (_isCompleted)
            {
                return;
            }
            _isCompleted = true;
            continuation = _continuation;
            _continuation = null;
        }
        continuation?.Invoke();
    }

    internal void TrySetException(Exception exception)
    {
        ArgumentNullException.ThrowIfNull(exception);
        Action? continuation = null;
        lock (this)
        {
            if (_isCompleted)
            {
                return;
            }
            _exception = exception;
            _isCompleted = true;
            continuation = _continuation;
            _continuation = null;
        }
        continuation?.Invoke();
    }

    internal void GetResult()
    {
        if (!_isCompleted)
        {
            throw new InvalidOperationException("Task has not completed.");
        }
        if (_exception is not null)
        {
            throw _exception;
        }
    }
}

[Stub(InPlaceOf = "System.Threading.Tasks.Task`1")]
public class Task<TResult> : Task
{
    [UsedImplicitly] private TResult? _result;
    [UsedImplicitly] private bool _hasResult;

    internal Task() : base() { }

    private Task(TResult result) : base(completed: true)
    {
        _result = result;
        _hasResult = true;
    }

    public TResult Result => GetResultCore();

    public new TaskAwaiter<TResult> GetAwaiter() => new(this);

    internal TResult GetResultCore()
    {
        base.GetResult();
        return _hasResult ? _result! : default!;
    }

    internal void TrySetResult(TResult result)
    {
        _result = result;
        _hasResult = true;
        base.TrySetResult();
    }

    public static Task<TResult> FromResult(TResult result) => new(result);

    public static Task<TResult> FromException(Exception exception)
    {
        ArgumentNullException.ThrowIfNull(exception);
        var task = new Task<TResult>();
        task.TrySetException(exception);
        return task;
    }
}
