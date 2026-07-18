using System.Security.Authentication;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Optional-TLS transport tests (SPEC-008 CAN-020, FR-29). Mirrors
/// rust/thunder/tests/tls.rs. Three properties: an encrypted round-trip works
/// end to end; the plaintext path is unchanged when TLS is unused; and a cert
/// the client does not trust fails as a <see cref="ThunderConnectionException"/>,
/// not a hang or a panic.
/// </summary>
public class TlsTests
{
    /// <summary>
    /// A no-handshake config with TLS <em>policy</em> Optional — the actual
    /// transport TLS is driven by the client/server configs, not this signal
    /// (PRO-003: config is data).
    /// </summary>
    private static Config TlsProfile() => new()
    {
        Scheme = "test",
        DefaultPort = 0,
        Handshake = Handshake.None,
        HelloStyle = HelloStyle.NotUsed,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = MockServer.ServerCap,
        MaxInFlight = 64,
        ErrorCodes = ErrorConvention.None,
        Tls = TlsPolicy.Optional,
    };

    /// <summary>A fresh self-signed cert/key for <c>localhost</c>, with a persistable key.</summary>
    private static X509Certificate2 SelfSigned(string commonName = "localhost")
    {
        using var rsa = RSA.Create(2048);
        var request = new CertificateRequest(
            $"CN={commonName}", rsa, HashAlgorithmName.SHA256, RSASignaturePadding.Pkcs1);
        var san = new SubjectAlternativeNameBuilder();
        san.AddDnsName(commonName);
        request.CertificateExtensions.Add(san.Build());
        // CA=true so the self-signed cert is usable as its own trust anchor when
        // the client pins it (the pinned-CA build treats it as a root).
        request.CertificateExtensions.Add(new X509BasicConstraintsExtension(true, false, 0, true));
        using var ephemeral = request.CreateSelfSigned(
            DateTimeOffset.UtcNow.AddMinutes(-5), DateTimeOffset.UtcNow.AddDays(1));
        // Round-trip through PFX so the private key is persistable — Windows
        // SslStream server auth rejects the ephemeral key otherwise.
        return new X509Certificate2(ephemeral.Export(X509ContentType.Pfx));
    }

    /// <summary>Write the certificate's public PEM to a unique temp file, returning its path.</summary>
    private static string WriteCertPem(X509Certificate2 certificate)
    {
        var path = Path.Combine(Path.GetTempPath(), $"thunder-tls-{Guid.NewGuid():N}.pem");
        File.WriteAllText(path, certificate.ExportCertificatePem());
        return path;
    }

    [Fact]
    public async Task Tls_round_trip_encrypts_request_and_response()
    {
        using var cert = SelfSigned();
        var caPath = WriteCertPem(cert);
        try
        {
            using var server = new MockServer();
            var serverTask = Task.Run(async () =>
            {
                using var conn = await server.AcceptTlsAsync(cert);
                var ping = await conn.ReadRequestAsync();
                await conn.SendOkAsync(ping.Id, Value.Str("PONG"));
                var echo = await conn.ReadRequestAsync();
                await conn.SendOkAsync(
                    echo.Id, echo.Args.Count > 0 ? echo.Args[0] : Value.Null);
            });

            // The client trusts exactly this self-signed cert and verifies the
            // SAN `localhost`.
            var clientConfig = new ClientConfig
            {
                Tls = new ClientTls { ServerName = "localhost", CaPath = caPath },
            };
            await using var client = await ThunderClient.ConnectAsync(
                server.Address, TlsProfile(), clientConfig);

            Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());
            Assert.Equal(
                "secret-over-tls",
                (await client.CallAsync("ECHO", new[] { Value.Str("secret-over-tls") })).AsStr());

            client.Close();
            await serverTask;
        }
        finally
        {
            File.Delete(caPath);
        }
    }

    [Fact]
    public async Task Plaintext_still_works_when_tls_is_unused()
    {
        // The same client/server stack, no TLS on either end — proves the
        // default path is unchanged.
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var ping = await conn.ReadRequestAsync();
            await conn.SendOkAsync(ping.Id, Value.Str("PONG"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, TlsProfile());
        Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());

        client.Close();
        await serverTask;
    }

    [Fact]
    public async Task Cert_mismatch_is_a_connection_error()
    {
        using var serverCert = SelfSigned();
        // A DIFFERENT self-signed cert the client trusts instead of the
        // server's — verification must fail.
        using var otherCert = SelfSigned();
        var wrongCaPath = WriteCertPem(otherCert);
        try
        {
            using var server = new MockServer();
            var serverTask = Task.Run(async () =>
            {
                try
                {
                    using var conn = await server.AcceptTlsAsync(serverCert);
                    await conn.ReadRequestAsync();
                }
                catch (Exception e) when (e is IOException or AuthenticationException)
                {
                    // The client aborts the TLS handshake on the untrusted cert.
                }
            });

            var clientConfig = new ClientConfig
            {
                Tls = new ClientTls { ServerName = "localhost", CaPath = wrongCaPath },
            };
            // FR-29: a TLS/verification failure is the Connection class.
            await Assert.ThrowsAsync<ThunderConnectionException>(
                () => ThunderClient.ConnectAsync(server.Address, TlsProfile(), clientConfig));

            await serverTask;
        }
        finally
        {
            File.Delete(wrongCaPath);
        }
    }
}
