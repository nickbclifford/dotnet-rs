using System;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;

public static class Program
{
    private static int s_continuationCount;

    private struct ProbeAwaiter : ICriticalNotifyCompletion
    {
        private Action? _continuation;

        public bool IsCompleted => false;
        public bool HasContinuation => _continuation is not null;

        public void OnCompleted(Action continuation)
        {
            _continuation = continuation;
        }

        public void UnsafeOnCompleted(Action continuation)
        {
            _continuation = continuation;
        }

        public void Resume()
        {
            Action? continuation = _continuation;
            _continuation = null;
            continuation?.Invoke();
        }

        public void GetResult() { }
    }

    private struct ProbeStateMachine : IAsyncStateMachine
    {
        public void MoveNext()
        {
            s_continuationCount++;
        }

        public void SetStateMachine(IAsyncStateMachine stateMachine) { }
    }

    public static int Main()
    {
        s_continuationCount = 0;
        var builder = AsyncValueTaskMethodBuilder<int>.Create();
        var stateMachine = new ProbeStateMachine();
        var awaiter = new ProbeAwaiter();

        builder.AwaitUnsafeOnCompleted(ref awaiter, ref stateMachine);
        if (s_continuationCount != 0)
        {
            return 1;
        }
        if (!awaiter.HasContinuation)
        {
            return 2;
        }
        awaiter.Resume();
        if (s_continuationCount != 1)
        {
            return 3;
        }

        awaiter = new ProbeAwaiter();
        builder.AwaitOnCompleted(ref awaiter, ref stateMachine);
        if (s_continuationCount != 1)
        {
            return 4;
        }
        if (!awaiter.HasContinuation)
        {
            return 5;
        }
        awaiter.Resume();
        if (s_continuationCount != 2)
        {
            return 6;
        }

        builder.SetResult(40);
        return builder.Task.Result + 2;
    }
}
