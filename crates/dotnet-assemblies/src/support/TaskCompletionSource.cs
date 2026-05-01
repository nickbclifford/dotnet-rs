using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Threading.Tasks.TaskCompletionSource`1")]
public class TaskCompletionSource<TResult>
{
    [UsedImplicitly] private readonly Task<TResult> _task = new();

    public Task<TResult> Task => _task;

    public void SetResult(TResult result)
    {
        if (!TrySetResult(result))
        {
            throw new InvalidOperationException("Task is already completed.");
        }
    }

    public bool TrySetResult(TResult result)
    {
        if (_task.IsCompleted)
        {
            return false;
        }

        _task.TrySetResult(result);
        return true;
    }

    public void SetException(Exception exception)
    {
        if (!TrySetException(exception))
        {
            throw new InvalidOperationException("Task is already completed.");
        }
    }

    public bool TrySetException(Exception exception)
    {
        if (exception is null)
        {
            throw new ArgumentNullException(nameof(exception));
        }

        if (_task.IsCompleted)
        {
            return false;
        }

        _task.TrySetException(exception);
        return true;
    }
}
