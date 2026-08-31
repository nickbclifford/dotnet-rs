using System.Runtime.InteropServices;
using System.Threading;

public class Program
{
    const int Rounds = 100;
    static CountdownEvent[] s_ready = new CountdownEvent[Rounds];
    static CountdownEvent[] s_checked = new CountdownEvent[Rounds];
    static volatile int s_workerFailure;

    static void Worker()
    {
        int failure = 0;

        for (int round = 0; round < Rounds; round++)
        {
            int value = 200000 + round;
            Marshal.SetLastPInvokeError(value);

            s_ready[round].Signal();
            while (!s_ready[round].IsSet)
            {
                Thread.Sleep(0);
            }

            if (Marshal.GetLastPInvokeError() != value)
            {
                failure = 1;
            }

            s_checked[round].Signal();
            while (!s_checked[round].IsSet)
            {
                Thread.Sleep(0);
            }
        }

        s_workerFailure = failure;
    }

    public static int Main()
    {
        for (int round = 0; round < Rounds; round++)
        {
            s_ready[round] = new CountdownEvent(2);
            s_checked[round] = new CountdownEvent(2);
        }

        var worker = new Thread(Worker);
        worker.Start();

        int failure = 0;
        for (int round = 0; round < Rounds; round++)
        {
            int value = 100000 + round;
            Marshal.SetLastPInvokeError(value);

            s_ready[round].Signal();
            while (!s_ready[round].IsSet)
            {
                Thread.Sleep(0);
            }

            if (Marshal.GetLastPInvokeError() != value)
            {
                failure = 1;
            }

            s_checked[round].Signal();
            while (!s_checked[round].IsSet)
            {
                Thread.Sleep(0);
            }
        }

        worker.Join();
        return failure == 0 && s_workerFailure == 0 ? 42 : 1;
    }
}
