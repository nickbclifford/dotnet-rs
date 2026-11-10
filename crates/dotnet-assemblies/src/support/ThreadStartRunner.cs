using System.Threading;

namespace DotnetRs;

internal static class ThreadStartRunner
{
    internal static void Run(ThreadStart start, object? argument) => start();
    internal static void RunParameterized(ParameterizedThreadStart start, object? argument) => start(argument);
}
