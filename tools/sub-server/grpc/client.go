package grpcclient

import (
	"context"
	"fmt"
	"net"
	"strings"
	"time"

	pb "github.com/NicholasDewar/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type Client struct {
	conn  *grpc.ClientConn
	sub   pb.SubscriptionServiceClient
	token pb.TokenServiceClient
}

func New(socketPath string) (*Client, error) {
	target := socketPath
	var opts []grpc.DialOption
	if socketPath == "" {
		return nil, fmt.Errorf("gRPC socket path is empty")
	}
	opts = append(opts, grpc.WithTransportCredentials(insecure.NewCredentials()))
	opts = append(opts, grpc.WithContextDialer(func(ctx context.Context, addr string) (net.Conn, error) {
		// Strip unix:// prefix if present (gRPC resolver may pass it through)
		path := strings.TrimPrefix(addr, "unix://")
		var d net.Dialer
		return d.DialContext(ctx, "unix", path)
	}))
	opts = append(opts, grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(10*1024*1024)))
	conn, err := grpc.NewClient(target, opts...)
	if err != nil {
		return nil, fmt.Errorf("grpc new client: %w", err)
	}
	return &Client{
		conn:  conn,
		sub:   pb.NewSubscriptionServiceClient(conn),
		token: pb.NewTokenServiceClient(conn),
	}, nil
}

func (c *Client) GetConfigs(token string) ([]*pb.ProxyConfig, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	resp, err := c.sub.GetConfigs(ctx, &pb.GetConfigsRequest{Token: token})
	if err != nil {
		return nil, err
	}
	return resp.Configs, nil
}

func (c *Client) GetTokenInfo(token string) (*pb.TokenInfo, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	resp, err := c.sub.GetTokenInfo(ctx, &pb.GetTokenInfoRequest{Token: token})
	if err != nil {
		return nil, err
	}
	return resp, nil
}

func (c *Client) CreateToken(label string, configIDs []string) (*pb.SubscriptionToken, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	resp, err := c.token.CreateToken(ctx, &pb.CreateTokenRequest{
		Label:     label,
		ConfigIds: configIDs,
	})
	if err != nil {
		return nil, err
	}
	return resp.Info, nil
}

func (c *Client) Close() error {
	return c.conn.Close()
}
