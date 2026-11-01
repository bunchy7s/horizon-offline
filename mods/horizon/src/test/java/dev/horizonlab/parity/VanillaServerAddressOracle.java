package dev.horizonlab.parity;

import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.util.Optional;
import net.minecraft.client.multiplayer.resolver.AddressCheck;
import net.minecraft.client.multiplayer.resolver.ResolvedServerAddress;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.client.multiplayer.resolver.ServerAddressResolver;
import net.minecraft.client.multiplayer.resolver.ServerNameResolver;
import net.minecraft.client.multiplayer.resolver.ServerRedirectHandler;

/** ConnectScreen.startConnecting uses the resolved socket's host and port for login. */
public final class VanillaServerAddressOracle {
    public static void main(String[] args) throws Exception {
        AddressCheck allow = new AddressCheck() {
            public boolean isAllowed(ResolvedServerAddress address) { return true; }
            public boolean isAllowed(ServerAddress address) { return true; }
        };
        ServerAddressResolver addresses = address -> {
            try {
                return Optional.of(ResolvedServerAddress.from(new InetSocketAddress(
                    InetAddress.getByAddress(address.getHost(), new byte[]{127, 0, 0, 1}),
                    address.getPort())));
            } catch (Exception error) {
                throw new RuntimeException(error);
            }
        };
        var constructor = ServerNameResolver.class.getDeclaredConstructor(
            ServerAddressResolver.class, ServerRedirectHandler.class, AddressCheck.class);
        constructor.setAccessible(true);
        for (String scenario : new String[]{"direct", "redirect", "explicit"}) {
            int port = scenario.equals("explicit") ? 25566 : 25565;
            ServerRedirectHandler redirect = address ->
                scenario.equals("redirect") && address.getPort() == 25565
                    ? Optional.of(new ServerAddress("backend.example.org.", 25570))
                    : Optional.empty();
            var resolved = constructor.newInstance(addresses, redirect, allow)
                .resolveAddress(new ServerAddress("play.example.org", port))
                .orElseThrow().asInetSocketAddress();
            System.out.println("server_address=" + scenario + ":" + resolved.getHostName()
                + ":" + resolved.getPort());
        }
    }
}
