interface IReadOnlyNavigation {
    int Value { get; }
}

sealed class Navigation : IReadOnlyNavigation {
    public int Value => 42;
}

interface IReadOnlyEntityType {
    IReadOnlyNavigation FindDeclaredNavigation(string name);
    IReadOnlyEntityType BaseType { get; }

    IReadOnlyNavigation FindNavigation(string name) {
        var declared = FindDeclaredNavigation(name);
        if (declared != null) {
            return declared;
        }

        var baseType = BaseType;
        return baseType == null ? null : baseType.FindNavigation(name);
    }
}

sealed class EntityType : IReadOnlyEntityType {
    private readonly Navigation _navigation = new Navigation();

    public IReadOnlyNavigation FindDeclaredNavigation(string name)
        => name == "ok" ? _navigation : null;

    public IReadOnlyEntityType BaseType => null;

    // Covariant wrapper: same name/params, narrower return type, delegates via interface callvirt.
    public Navigation FindNavigation(string name)
        => (Navigation)((IReadOnlyEntityType)this).FindNavigation(name);
}

public static class Program {
    public static int Main() {
        var entityType = new EntityType();
        var navigation = entityType.FindNavigation("ok");
        return navigation != null && navigation.Value == 42 ? 42 : 0;
    }
}
