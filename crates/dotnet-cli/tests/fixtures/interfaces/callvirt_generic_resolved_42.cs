public interface IProjector<T>
{
    T Project(T value);
}

public abstract class ProjectorBase<T> : IProjector<T>
{
    public virtual T Project(T value) => value;
}

public sealed class OffsetProjector : ProjectorBase<int>
{
    public override int Project(int value) => value + 21;
}

public class Program
{
    private static T Invoke<T>(IProjector<T> projector, T value)
    {
        // The interface callvirt must carry the method resolved for T through virtual dispatch.
        return projector.Project(value);
    }

    public static int Main()
    {
        IProjector<int> projector = new OffsetProjector();
        return Invoke(projector, 21) == 42 ? 42 : 1;
    }
}
